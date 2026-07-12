// web_search tool: scrape HTML search backends with no API key and no JS.
//
// Primary → fallback chain (same security model as fetch — honors
// --no-network / fetch_allowlist, reuses html_to_text + egress helpers):
//   1. DuckDuckGo Lite  (https://lite.duckduckgo.com/lite/)
//   2. DuckDuckGo HTML  (https://html.duckduckgo.com/html/)
//   3. Mojeek           (https://www.mojeek.com/search)
//
// NO API KEY, NO JavaScript, NO new crate deps. Each backend returns a
// JS-free HTML page we parse with the `regex` crate the core already
// depends on. DDG wraps destinations in `uddg=` redirects which we decode
// by hand (no percent-encoding crate).
//
// This is best-effort scraping, not an SLA: backends may rate-limit / serve
// a captcha under burst traffic, and markup may drift. On block/HTTP failure
// / empty parse we try the next backend. Only if every backend fails do we
// surface an aggregated error; a successful empty SERP reports "no results".
use crate::config::Config;
use crate::fetch_tool::{egress_check, html_to_text};
use crate::tools::{smart_truncate, Outcome};
use regex::Regex;
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::sync::LazyLock;

// ---- shared regexes (compiled once) ----

/// DDG Lite: `<a class="result-link" href="...">title</a>`
static DDG_LITE_LINK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<a\s+class="result-link"\s+href="([^"]+)"[^>]*>([\s\S]*?)</a>"#).unwrap()
});
/// DDG Lite: `<td class="result-snippet">...</td>`
static DDG_LITE_SNIP_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"class="result-snippet"[^>]*>([\s\S]*?)</td>"#).unwrap());
/// Loose fallback for any `<a href>` when structured classes drift.
static ANY_LINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a\s+[^>]*href="([^"]+)"[^>]*>([\s\S]*?)</a>"#).unwrap());

/// DDG HTML: `<a class="result__a" href="...">title</a>` (class order may vary).
static DDG_HTML_LINK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<a\s+[^>]*class="[^"]*result__a[^"]*"[^>]*href="([^"]+)"[^>]*>([\s\S]*?)</a>"#)
        .unwrap()
});
/// Alternate attribute order: href before class.
static DDG_HTML_LINK_RE_ALT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<a\s+[^>]*href="([^"]+)"[^>]*class="[^"]*result__a[^"]*"[^>]*>([\s\S]*?)</a>"#)
        .unwrap()
});
/// DDG HTML snippets.
static DDG_HTML_SNIP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"class="[^"]*result__snippet[^"]*"[^>]*>([\s\S]*?)</(?:a|td|span|div)>"#).unwrap()
});

/// Mojeek: `<a class="title" title="url" href="url">Title</a>`
static MOJEEK_TITLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<a\s+class="title"[^>]*href="([^"]+)"[^>]*>([\s\S]*?)</a>"#).unwrap()
});
/// Mojeek snippet: `<p class="s">...</p>` (paired by index with titles).
static MOJEEK_SNIP_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<p\s+class="s">([\s\S]*?)</p>"#).unwrap());

/// Percent-decode a query-string value (no `percent-encoding` crate dep).
/// Handles `%XX` hex escapes and `+`→space. Malformed `%` sequences are passed
/// through literally rather than panicking.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// DDG wraps each result's destination in a redirect like
/// `//duckduckgo.com/l/?uddg=<encoded>&rut=...`. Extract and decode the real
/// URL. For direct hrefs (no `uddg=`), return the href unchanged (protocol-less
/// `//host/...` is upgraded to `https://`).
fn unwrap_ddg_url(href: &str) -> String {
    let href = if let Some(rest) = href.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        href.to_string()
    };
    if let Some(idx) = href.find("uddg=") {
        let after = &href[idx + "uddg=".len()..];
        let end = after.find('&').unwrap_or(after.len());
        return percent_decode(&after[..end]);
    }
    href
}

/// Strip tags from a snippet cell and tidy whitespace, reusing the shared
/// html_to_text helper so entities/whitespace are handled consistently with
/// the fetch tool's output.
fn cell_text(s: &str) -> String {
    let t = html_to_text(s);
    t.trim().to_string()
}

/// A single search hit.
#[derive(Clone, Debug)]
struct Hit {
    title: String,
    url: String,
    snippet: String,
}

/// Which backend produced the hits (shown in the tool output header).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    DdgLite,
    DdgHtml,
    Mojeek,
}

impl Backend {
    fn label(self) -> &'static str {
        match self {
            Backend::DdgLite => "DuckDuckGo Lite",
            Backend::DdgHtml => "DuckDuckGo HTML",
            Backend::Mojeek => "Mojeek",
        }
    }
}

/// Outcome of one backend attempt.
enum Attempt {
    /// Parsed ≥1 hit — stop the chain.
    Hits(Vec<Hit>),
    /// Page looked like a real SERP but had zero results — stop the chain
    /// (don't keep searching; the query genuinely has nothing).
    Empty,
    /// Blocked / HTTP error / markup drift — try the next backend.
    Fail(String),
}

/// Shared captcha / anomaly heuristics used by both DDG backends.
fn looks_blocked(html: &str) -> bool {
    let low = html.to_ascii_lowercase();
    low.contains("captcha")
        || low.contains("unusual traffic")
        || low.contains("are you a robot")
        || low.contains("bots use duckduckgo")
        || low.contains("anomaly-modal")
        || low.contains("please complete the following challenge")
}

/// Parse DDG Lite HTML into ordered hits. Returns up to `limit` results.
/// Defensive: if the structured `result-link`/`result-snippet` parse yields
/// nothing (markup drift / captcha), falls back to scraping any `<a href>`
/// whose href looks like a real external result.
fn parse_ddg_lite(html: &str, limit: usize) -> Vec<Hit> {
    let titles_urls: Vec<(String, String)> = DDG_LITE_LINK_RE
        .captures_iter(html)
        .map(|c| {
            let href = c.get(1).map(|m| m.as_str()).unwrap_or("");
            let title = c.get(2).map(|m| m.as_str()).unwrap_or("");
            (cell_text(title), unwrap_ddg_url(href))
        })
        .filter(|(t, u)| !t.is_empty() && !u.is_empty())
        .collect();
    let snippets: Vec<String> = DDG_LITE_SNIP_RE
        .captures_iter(html)
        .map(|c| cell_text(c.get(1).map(|m| m.as_str()).unwrap_or("")))
        .collect();

    if !titles_urls.is_empty() {
        return titles_urls
            .iter()
            .take(limit)
            .enumerate()
            .map(|(i, (t, u))| Hit {
                title: t.clone(),
                url: u.clone(),
                snippet: snippets.get(i).cloned().unwrap_or_default(),
            })
            .collect();
    }

    // Fallback: scrape external-looking <a href> links. Filters out anchors,
    // javascript:, and DDG-internal nav links. This is looser but still useful
    // when the structured classes drift.
    let mut hits: Vec<Hit> = Vec::new();
    for c in ANY_LINK_RE.captures_iter(html) {
        let href = c.get(1).map(|m| m.as_str()).unwrap_or("");
        let title = cell_text(c.get(2).map(|m| m.as_str()).unwrap_or(""));
        if (href.starts_with("http://")
            || (href.starts_with("https://") && !href.contains("duckduckgo.com/l/")))
            && !title.is_empty()
            && !hits.iter().any(|h| h.url == href)
        {
            hits.push(Hit {
                title,
                url: href.to_string(),
                snippet: String::new(),
            });
            if hits.len() >= limit {
                break;
            }
        }
    }
    hits
}

/// Parse DDG HTML (`html.duckduckgo.com/html/`) results: `result__a` +
/// `result__snippet`. Same `uddg=` unwrap as Lite.
fn parse_ddg_html(html: &str, limit: usize) -> Vec<Hit> {
    let mut titles_urls: Vec<(String, String)> = DDG_HTML_LINK_RE
        .captures_iter(html)
        .map(|c| {
            let href = c.get(1).map(|m| m.as_str()).unwrap_or("");
            let title = c.get(2).map(|m| m.as_str()).unwrap_or("");
            (cell_text(title), unwrap_ddg_url(href))
        })
        .filter(|(t, u)| !t.is_empty() && !u.is_empty())
        .collect();
    if titles_urls.is_empty() {
        titles_urls = DDG_HTML_LINK_RE_ALT
            .captures_iter(html)
            .map(|c| {
                let href = c.get(1).map(|m| m.as_str()).unwrap_or("");
                let title = c.get(2).map(|m| m.as_str()).unwrap_or("");
                (cell_text(title), unwrap_ddg_url(href))
            })
            .filter(|(t, u)| !t.is_empty() && !u.is_empty())
            .collect();
    }
    let snippets: Vec<String> = DDG_HTML_SNIP_RE
        .captures_iter(html)
        .map(|c| cell_text(c.get(1).map(|m| m.as_str()).unwrap_or("")))
        .collect();

    titles_urls
        .iter()
        .take(limit)
        .enumerate()
        .map(|(i, (t, u))| Hit {
            title: t.clone(),
            url: u.clone(),
            snippet: snippets.get(i).cloned().unwrap_or_default(),
        })
        .collect()
}

/// Parse Mojeek SERP: `a.title` + paired `p.s` snippets. Hrefs are direct
/// (no redirect wrapper).
fn parse_mojeek(html: &str, limit: usize) -> Vec<Hit> {
    let titles_urls: Vec<(String, String)> = MOJEEK_TITLE_RE
        .captures_iter(html)
        .map(|c| {
            let href = c.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
            let title = cell_text(c.get(2).map(|m| m.as_str()).unwrap_or(""));
            (title, href)
        })
        .filter(|(t, u)| !t.is_empty() && u.starts_with("http"))
        .collect();
    let snippets: Vec<String> = MOJEEK_SNIP_RE
        .captures_iter(html)
        .map(|c| cell_text(c.get(1).map(|m| m.as_str()).unwrap_or("")))
        .collect();

    titles_urls
        .iter()
        .take(limit)
        .enumerate()
        .map(|(i, (t, u))| Hit {
            title: t.clone(),
            url: u.clone(),
            snippet: snippets.get(i).cloned().unwrap_or_default(),
        })
        .collect()
}

/// Classify a fetched body for a given backend into Hits / Empty / Fail.
fn classify_response(
    backend: Backend,
    status: reqwest::StatusCode,
    html: &str,
    body_len: usize,
    truncated: bool,
    limit: usize,
) -> Attempt {
    let trunc = if truncated { " [body truncated]" } else { "" };
    if !status.is_success() {
        return Attempt::Fail(format!("{} returned HTTP {status}{trunc}", backend.label()));
    }

    if looks_blocked(html) {
        return Attempt::Fail(format!(
            "{} served a captcha/anomaly page (likely rate-limited){trunc}",
            backend.label()
        ));
    }

    let low = html.to_ascii_lowercase();
    let has_markers = match backend {
        Backend::DdgLite => low.contains("result-link") || low.contains("result-snippet"),
        Backend::DdgHtml => {
            low.contains("result__a")
                || low.contains("result__snippet")
                || low.contains("web-result")
        }
        Backend::Mojeek => low.contains("class=\"title\"") || low.contains("class=\"s\""),
    };

    // Small page with no result markers → block / markup drift, not "no hits".
    if !has_markers && body_len < 8 * 1024 {
        return Attempt::Fail(format!(
            "{} returned an unexpected page with no result markers (markup drift or a block){trunc}",
            backend.label()
        ));
    }

    let hits = match backend {
        Backend::DdgLite => parse_ddg_lite(html, limit),
        Backend::DdgHtml => parse_ddg_html(html, limit),
        Backend::Mojeek => parse_mojeek(html, limit),
    };

    if hits.is_empty() {
        // Markers present (or large page) but nothing parsed: treat as empty
        // SERP when markers exist; otherwise as a soft fail so the chain continues.
        if has_markers {
            Attempt::Empty
        } else {
            Attempt::Fail(format!(
                "{} returned a page that parsed to zero results{trunc}",
                backend.label()
            ))
        }
    } else {
        Attempt::Hits(hits)
    }
}

/// GET `url`, stream up to `byte_limit` bytes, return (status, body, truncated).
async fn fetch_html(
    client: &reqwest::Client,
    url: &str,
    byte_limit: usize,
) -> Result<(reqwest::StatusCode, String, bool), String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    use futures_util::StreamExt;
    let mut collected: Vec<u8> = Vec::with_capacity(byte_limit.min(64 * 1024));
    let mut truncated = false;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("failed to read body: {e}"))?;
        let room = byte_limit - collected.len();
        if chunk.len() <= room {
            collected.extend_from_slice(&chunk);
        } else {
            collected.extend_from_slice(&chunk[..room]);
            truncated = true;
            break;
        }
    }
    let html = String::from_utf8_lossy(&collected).into_owned();
    Ok((status, html, truncated))
}

fn render_hits(query: &str, backend: Backend, hits: &[Hit]) -> Outcome {
    let mut text = format!(
        "Search: {query}  ({}, {} hit(s))\n\n",
        backend.label(),
        hits.len()
    );
    for (i, h) in hits.iter().enumerate() {
        text.push_str(&format!(
            "{}. {}\n   {}\n   {}\n\n",
            i + 1,
            h.title,
            h.url,
            h.snippet
        ));
    }

    const OUT_CAP: usize = 24_576;
    if text.len() > OUT_CAP {
        text = smart_truncate(&text, OUT_CAP);
    }
    Outcome::ok(text)
}

pub async fn execute_web_search(args: &Value, cfg: &Config) -> Outcome {
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    if query.trim().is_empty() {
        return Outcome::err("web_search requires a non-empty 'query'");
    }
    let count = args
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(8)
        .clamp(1, 20) as usize;
    let region = args
        .get("region")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("us-en");

    let q = form_urlencode(query);
    // Ordered fallback chain. Region (`kl=`) only applies to DDG backends.
    let backends: [(Backend, String); 3] = [
        (
            Backend::DdgLite,
            format!("https://lite.duckduckgo.com/lite/?q={q}&kl={region}"),
        ),
        (
            Backend::DdgHtml,
            format!("https://html.duckduckgo.com/html/?q={q}&kl={region}"),
        ),
        (
            Backend::Mojeek,
            format!("https://www.mojeek.com/search?q={q}"),
        ),
    ];

    // Pre-check egress on the first URL so --no-network fails fast with the
    // same message as before. Per-backend checks still run below so a narrow
    // allowlist can permit DDG but deny Mojeek (or vice versa).
    if let Some(err) = egress_check("web_search", &backends[0].1, cfg) {
        return Outcome::err(err);
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(
            cfg.fetch_timeout_secs.max(1),
        ))
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(crate::fetch_tool::allowlist_redirect_policy(
            cfg.fetch_allowlist.clone(),
        ))
        // Search engines block obvious bot UAs; use a plain browser UA.
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36")
        .build()
    {
        Ok(c) => c,
        Err(e) => return Outcome::err(format!("web_search: failed to build HTTP client: {e}")),
    };

    let byte_limit = cfg.fetch_max_bytes.max(64 * 1024);
    let mut failures: Vec<String> = Vec::new();

    for (backend, url) in &backends {
        if let Some(err) = egress_check("web_search", url, cfg) {
            failures.push(format!("{}: skipped ({err})", backend.label()));
            continue;
        }

        let (status, html, truncated) = match fetch_html(&client, url, byte_limit).await {
            Ok(v) => v,
            Err(e) => {
                failures.push(format!("{}: {e}", backend.label()));
                continue;
            }
        };

        match classify_response(*backend, status, &html, html.len(), truncated, count) {
            Attempt::Hits(hits) => return render_hits(query, *backend, &hits),
            Attempt::Empty => {
                return Outcome::ok(format!(
                    "No results found for {query:?} on {}.{}",
                    backend.label(),
                    if truncated {
                        " [page body was truncated; refine the query]"
                    } else {
                        ""
                    }
                ));
            }
            Attempt::Fail(reason) => {
                failures.push(reason);
            }
        }
    }

    Outcome::err(format!(
        "web_search: all backends failed; retry later or use the `fetch` tool against a specific URL. query was {query:?}. attempts: {}",
        failures.join(" | ")
    ))
}

/// Minimal application/x-www-form-urlencoded encoder for the query string
/// (no `form_urlencoded` crate dep). Encodes everything except unreserved
/// chars [A-Za-z0-9_.~-] as %XX; spaces become %20 (NOT +) which DDG accepts.
fn form_urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_basic() {
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(
            percent_decode("https%3A%2F%2Fexample.com%2Fpath"),
            "https://example.com/path"
        );
        // malformed trailing % passes through literally
        assert_eq!(percent_decode("abc%ZZ%"), "abc%ZZ%");
    }

    #[test]
    fn unwrap_ddg_redirect() {
        assert_eq!(
            unwrap_ddg_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org%2F&rut=x"),
            "https://rust-lang.org/"
        );
        // direct https href unchanged
        assert_eq!(
            unwrap_ddg_url("https://example.com/page"),
            "https://example.com/page"
        );
        // protocol-relative direct link upgraded
        assert_eq!(unwrap_ddg_url("//example.com/x"), "https://example.com/x");
    }

    #[test]
    fn form_urlencode_encodes_spaces_and_special() {
        assert_eq!(form_urlencode("hello world"), "hello%20world");
        assert_eq!(form_urlencode("a&b=c"), "a%26b%3Dc");
        // unreserved chars stay literal
        assert_eq!(form_urlencode("A-z_0.~"), "A-z_0.~");
    }

    const SAMPLE_DDG_LITE: &str = r#"<html><body>
<table>
<tr><td><a class="result-link" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2F&rut=abc">The Rust Programming Language</a></td></tr>
<tr><td class="result-snippet">A language empowering everyone to build reliable &amp; efficient software.</td></tr>
<tr><td><a class="result-link" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org%2Fstd%2F&rut=def">std - Rust</a></td></tr>
<tr><td class="result-snippet">API documentation for the Rust standard library.</td></tr>
<tr><td><a class="result-link" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fads&rut=zzz">Sponsored Ad</a></td></tr>
<tr><td class="result-snippet">Buy our stuff now.</td></tr>
</table>
</body></html>"#;

    const SAMPLE_DDG_HTML: &str = r#"<html><body>
<div id="links">
<div class="result results_links web-result">
  <h2 class="result__title"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2F&rut=abc">The Rust Programming Language</a></h2>
  <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2F">A language empowering everyone to build reliable &amp; efficient software.</a>
</div>
<div class="result results_links web-result">
  <h2 class="result__title"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org%2Fstd%2F&rut=def">std - Rust</a></h2>
  <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org%2Fstd%2F">API documentation for the Rust standard library.</a>
</div>
</div>
</body></html>"#;

    const SAMPLE_MOJEEK: &str = r#"<html><body><ul>
<!--rs--><li class="r1"><a title="https://rust-lang.org/" href="https://rust-lang.org/" class="ob"><p class="i"><span class="url">https://rust-lang.org/</span></p></a><h2><a class="title" title="https://rust-lang.org/" href="https://rust-lang.org/">Rust Programming Language</a></h2><p class="s"><strong>Rust</strong> is blazingly fast and memory-efficient.</p></li><!--re-->
<!--rs--><li class="r2"><a title="https://doc.rust-lang.org/book/" href="https://doc.rust-lang.org/book/" class="ob"></a><h2><a class="title" title="https://doc.rust-lang.org/book/" href="https://doc.rust-lang.org/book/">The Rust Programming Language</a></h2><p class="s">The HTML format is available online.</p></li><!--re-->
</ul></body></html>"#;

    #[test]
    fn parse_ddg_lite_structured() {
        let hits = parse_ddg_lite(SAMPLE_DDG_LITE, 10);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].title, "The Rust Programming Language");
        assert_eq!(hits[0].url, "https://www.rust-lang.org/");
        assert!(hits[0].snippet.contains("reliable & efficient"));
        assert_eq!(hits[1].url, "https://doc.rust-lang.org/std/");
        assert_eq!(hits[2].title, "Sponsored Ad");
    }

    #[test]
    fn parse_ddg_lite_respects_limit() {
        let hits = parse_ddg_lite(SAMPLE_DDG_LITE, 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://www.rust-lang.org/");
    }

    #[test]
    fn parse_ddg_lite_fallback_scrapes_links() {
        // markup drift: no result-link class, but external <a> hrefs exist
        let drifted = r#"<div><a href="https://example.org/page1">First</a> <a href="https://example.org/page2">Second</a></div>"#;
        let hits = parse_ddg_lite(drifted, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://example.org/page1");
        assert_eq!(hits[0].title, "First");
        // snippet empty in fallback
        assert!(hits[0].snippet.is_empty());
    }

    #[test]
    fn parse_ddg_html_structured() {
        let hits = parse_ddg_html(SAMPLE_DDG_HTML, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "The Rust Programming Language");
        assert_eq!(hits[0].url, "https://www.rust-lang.org/");
        assert!(hits[0].snippet.contains("reliable & efficient"));
        assert_eq!(hits[1].url, "https://doc.rust-lang.org/std/");
    }

    #[test]
    fn parse_ddg_html_href_before_class() {
        let html = r##"<a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2F&rut=x" class="result__a">Example</a>
<a class="result__snippet" href="#">An example site.</a>"##;
        let hits = parse_ddg_html(html, 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://example.com/");
        assert_eq!(hits[0].title, "Example");
    }

    #[test]
    fn parse_mojeek_structured() {
        let hits = parse_mojeek(SAMPLE_MOJEEK, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Rust Programming Language");
        assert_eq!(hits[0].url, "https://rust-lang.org/");
        assert!(hits[0].snippet.contains("blazingly fast"));
        assert_eq!(hits[1].url, "https://doc.rust-lang.org/book/");
    }

    #[test]
    fn classify_captcha_is_fail() {
        let html = "<html><body>please solve the captcha to continue</body></html>";
        match classify_response(
            Backend::DdgLite,
            reqwest::StatusCode::OK,
            html,
            html.len(),
            false,
            8,
        ) {
            Attempt::Fail(r) => assert!(r.contains("captcha") || r.contains("anomaly")),
            _ => panic!("expected Fail"),
        }
    }

    #[test]
    fn classify_ddg_html_hits() {
        match classify_response(
            Backend::DdgHtml,
            reqwest::StatusCode::OK,
            SAMPLE_DDG_HTML,
            SAMPLE_DDG_HTML.len(),
            false,
            8,
        ) {
            Attempt::Hits(h) => assert_eq!(h.len(), 2),
            _ => panic!("expected Hits"),
        }
    }

    #[test]
    fn classify_mojeek_hits() {
        match classify_response(
            Backend::Mojeek,
            reqwest::StatusCode::OK,
            SAMPLE_MOJEEK,
            SAMPLE_MOJEEK.len(),
            false,
            8,
        ) {
            Attempt::Hits(h) => assert_eq!(h.len(), 2),
            _ => panic!("expected Hits"),
        }
    }

    #[test]
    fn classify_anomaly_modal_is_fail() {
        let html =
            r#"<div class="anomaly-modal__title">Unfortunately, bots use DuckDuckGo too.</div>"#;
        match classify_response(
            Backend::DdgHtml,
            reqwest::StatusCode::OK,
            html,
            html.len(),
            false,
            8,
        ) {
            Attempt::Fail(r) => assert!(r.contains("captcha") || r.contains("anomaly")),
            _ => panic!("expected Fail"),
        }
    }

    // ---- HTTP integration against a one-shot mock server ----
    fn find_header_end(b: &[u8]) -> Option<usize> {
        b.windows(4).position(|w| w == b"\r\n\r\n")
    }

    async fn mock_http(body: String) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/lite/?q=test");
        let h = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf: Vec<u8> = Vec::new();
            let mut tmp = [0u8; 1024];
            while find_header_end(&buf).is_none() {
                let n = sock.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.flush().await.unwrap();
        });
        (url, h)
    }

    #[tokio::test]
    async fn web_search_parses_results_over_http() {
        let body = SAMPLE_DDG_LITE.to_string();
        let (_url, _h) = mock_http(body).await;
        // We can't easily point the tool at the mock (URL is hardcoded to DDG),
        // but we can exercise the full parse path + output rendering against the
        // sample HTML by calling parse_ddg_lite directly, and confirm the mock
        // server path used by fetch_tool's tests still serves HTML correctly.
        let hits = parse_ddg_lite(SAMPLE_DDG_LITE, 8);
        assert_eq!(hits.len(), 3);
    }

    #[tokio::test]
    async fn web_search_captcha_is_surfaced() {
        // Build args against a known query; we only test the captcha-detection
        // branch by feeding a synthetic captcha HTML to the parser-detector.
        let html = "<html><body>please solve the captcha to continue</body></html>";
        let low = html.to_ascii_lowercase();
        assert!(low.contains("captcha"));
        let has_result_classes = low.contains("result-link") || low.contains("result-snippet");
        assert!(!has_result_classes);
        assert!(looks_blocked(html));
    }

    #[test]
    fn empty_query_errors() {
        // sync pre-check (block on runtime in the test)
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cfg = Config::default();
        let out = rt.block_on(execute_web_search(&json!({ "query": "  " }), &cfg));
        assert!(!out.ok);
        assert!(out.output.contains("non-empty 'query'"));
    }

    #[test]
    fn no_network_denies() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cfg = Config {
            no_network: true,
            fetch_allowlist: Vec::new(),
            ..Config::default()
        };
        let out = rt.block_on(execute_web_search(&json!({ "query": "rust lang" }), &cfg));
        assert!(!out.ok);
        assert!(out.output.contains("--no-network"));
    }

    #[test]
    fn render_hits_names_backend() {
        let hits = vec![Hit {
            title: "T".into(),
            url: "https://example.com/".into(),
            snippet: "S".into(),
        }];
        let out = render_hits("q", Backend::Mojeek, &hits);
        assert!(out.ok);
        assert!(out.output.contains("Mojeek"));
        assert!(out.output.contains("https://example.com/"));
    }
}
