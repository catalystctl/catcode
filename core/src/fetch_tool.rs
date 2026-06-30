// Native HTTP fetch tool. Unlike `bash curl`, this is a first-class tool that is
// NOT subject to the bash sandbox / `--no-network` (`unshare -n` only wraps the
// bash command), so the agent can still look up docs under the hard-security
// config. A host allowlist (`cfg.fetch_allowlist`) restricts egress; empty
// allowlist = any http(s) host.
use crate::config::Config;
use crate::tools::{smart_truncate, Outcome};
use serde_json::Value;

/// Parse an absolute http(s) URL enough to validate it and extract the host
/// (lowercased) for allowlist matching. Returns (scheme, host). No `url` crate
/// dependency — a tiny hand-rolled parse is plenty and avoids a new dep.
fn parse_http_host(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let end = rest
        .find(&['/', '?', '#', ':'][..])
        .unwrap_or(rest.len());
    let host = rest[..end].to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some((scheme, host))
}

/// Match a host against one allowlist glob pattern. A leading `*.` matches the
/// bare domain and any subdomain (`*.rust-lang.org` matches `rust-lang.org`
/// and `doc.rust-lang.org`); otherwise exact, case-insensitive.
fn host_matches(host: &str, pattern: &str) -> bool {
    let h = host.to_ascii_lowercase();
    let p = pattern.trim().to_ascii_lowercase();
    if let Some(rest) = p.strip_prefix("*.") {
        h == rest || h.ends_with(&format!(".{rest}"))
    } else {
        h == p
    }
}

fn host_allowed(host: &str, allowlist: &[String]) -> bool {
    if allowlist.is_empty() {
        return true;
    }
    allowlist.iter().any(|p| host_matches(host, p))
}

/// Remove every case-insensitive `<open ...>...</open>` block from `s`
/// (used to drop `<script>`/`<style>` so their text doesn't leak into the
/// readable output). Bounded bodies keep this affordable despite the
/// repeated lowercase scans.
fn cut_blocks(s: &str, open: &str, close: &str) -> String {
    let open_l = open.to_ascii_lowercase();
    let close_l = close.to_ascii_lowercase();
    let close_len = close_l.len();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < s.len() {
        let lower = s[i..].to_ascii_lowercase();
        match lower.find(&open_l) {
            Some(p) => {
                out.push_str(&s[i..i + p]);
                let after = i + p;
                let rest_l = s[after..].to_ascii_lowercase();
                match rest_l.find(&close_l) {
                    Some(c) => i = after + c + close_len,
                    None => break, // no closer: drop the rest
                }
            }
            None => {
                out.push_str(&s[i..]);
                break;
            }
        }
    }
    out
}

/// Collapse runs of whitespace, preserving newlines (a doc page stays
/// paragraph-shaped instead of one giant line). Uses `char::from(10)` for the
/// newline to keep this file free of backslash escapes.
fn collapse_ws(s: &str) -> String {
    let nl = char::from(10);
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = false;
    let mut prev_nl = false;
    for c in s.chars() {
        if c == nl {
            if !prev_nl {
                out.push(nl);
            }
            prev_nl = true;
            prev_ws = false;
            continue;
        }
        prev_nl = false;
        if c.is_whitespace() {
            if !prev_ws {
                out.push(' ');
            }
            prev_ws = true;
        } else {
            out.push(c);
            prev_ws = false;
        }
    }
    out
}

/// Decode the handful of HTML entities that show up in doc text. `&amp;` is
/// decoded last so `&amp;lt;` becomes `&lt;`, not `<`. Uses `char::from(n)` for
/// the quote chars to avoid backslash/quote escaping in source.
fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&quot;", &char::from(34).to_string())
        .replace("&#39;", &char::from(39).to_string())
        .replace("&apos;", &char::from(39).to_string())
        .replace("&amp;", "&")
}

/// Very light HTML-to-text: drop script/style blocks, strip tags, collapse
/// whitespace, decode common entities. Not a real parser — just enough that a
/// fetched doc page is readable instead of a wall of markup.
fn html_to_text(html: &str) -> String {
    let s = cut_blocks(html, "<script", "</script>");
    let s = cut_blocks(&s, "<style", "</style>");
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(&collapse_ws(&out))
}

pub async fn execute_fetch(args: &Value, cfg: &Config) -> Outcome {
    let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
    let raw = args.get("raw").and_then(|v| v.as_bool()).unwrap_or(false);
    if url.is_empty() {
        return Outcome::err("fetch requires a 'url'");
    }
    let (_scheme, host) = match parse_http_host(url) {
        Some(v) => v,
        None => return Outcome::err("fetch: url must be an absolute http(s) URL"),
    };
    // --no-network blocks bash egress; honor the operator's intent for fetch
    // too, UNLESS they explicitly configured a fetch_allowlist (opting specific
    // hosts in for doc lookups while keeping bash offline). Empty allowlist +
    // --no-network = deny (no surprise bypass of the egress block).
    if cfg.no_network && cfg.fetch_allowlist.is_empty() {
        return Outcome::err(
            "fetch: network egress is disabled (--no-network) and no fetch_allowlist is configured; populate fetch_allowlist to opt specific hosts in for fetch while keeping bash offline",
        );
    }
    if !host_allowed(&host, &cfg.fetch_allowlist) {
        return Outcome::err(format!(
            "fetch: host '{host}' is not in the allowlist ({} pattern(s) configured); add it to fetch_allowlist to permit it, or leave the allowlist empty to allow any host",
            cfg.fetch_allowlist.len()
        ));
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(cfg.fetch_timeout_secs.max(1)))
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("umans-harness-fetch/0.1")
        .build()
    {
        Ok(c) => c,
        Err(e) => return Outcome::err(format!("fetch: failed to build HTTP client: {e}")),
    };

    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => return Outcome::err(format!("fetch: request failed: {e}")),
    };
    let status = resp.status();
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    // Stream the body, bounding memory to `limit + 1` so a giant response can't
    // OOM the agent. The +1 lets us detect truncation.
    let limit = cfg.fetch_max_bytes.max(1024);
    use futures_util::StreamExt;
    let mut collected: Vec<u8> = Vec::with_capacity(limit.min(64 * 1024));
    let mut truncated = false;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => return Outcome::err(format!("fetch: failed to read body: {e}")),
        };
        let room = limit - collected.len();
        if chunk.len() <= room {
            collected.extend_from_slice(&chunk);
        } else {
            collected.extend_from_slice(&chunk[..room]);
            truncated = true;
            break;
        }
    }

    let is_html = !raw && (ctype.contains("text/html") || ctype.starts_with("application/xhtml"));
    let text = if is_html {
        html_to_text(&String::from_utf8_lossy(&collected))
    } else {
        String::from_utf8_lossy(&collected).to_string()
    };

    let mut out = format!("HTTP {status}  {ctype}\n");
    if truncated {
        out.push_str(&format!("...[body truncated at {limit} bytes]...\n"));
    }
    out.push_str(&text);

    // Cap the final text the model sees so a big doc doesn't blow context.
    const OUT_CAP: usize = 65_536;
    if out.len() > OUT_CAP {
        out = smart_truncate(&out, OUT_CAP);
    }
    Outcome {
        ok: status.is_success(),
        output: out,
        diff: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_host_basic() {
        assert_eq!(
            parse_http_host("https://Doc.Rust-Lang.org/std/"),
            Some(("https".into(), "doc.rust-lang.org".into()))
        );
        assert_eq!(
            parse_http_host("http://example.com:8080/x"),
            Some(("http".into(), "example.com".into()))
        );
        assert_eq!(parse_http_host("file:///etc/passwd"), None);
        assert_eq!(parse_http_host("not a url"), None);
    }

    #[test]
    fn host_allowlist_matching() {
        assert!(host_matches("doc.rust-lang.org", "*.rust-lang.org"));
        assert!(host_matches("rust-lang.org", "*.rust-lang.org"));
        assert!(!host_matches("evilrust-lang.org", "*.rust-lang.org"));
        assert!(host_matches("docs.rs", "docs.rs"));
        assert!(!host_matches("docs.rs", "crates.io"));
        // empty allowlist = allow all
        assert!(host_allowed("anything.example", &[]));
        let list = vec!["*.rust-lang.org".into(), "docs.rs".into()];
        assert!(host_allowed("doc.rust-lang.org", &list));
        assert!(host_allowed("docs.rs", &list));
        assert!(!host_allowed("evil.com", &list));
    }

    #[test]
    fn html_strips_tags_and_scripts() {
        let html = "<html><head><style>body{color:red}</style></head><body><script>alert(1)</script><h1>Title</h1><p>Hi &amp; bye &lt;3</p></body></html>";
        let text = html_to_text(html);
        assert!(!text.contains("alert"));
        assert!(!text.contains("color:red"));
        assert!(text.contains("Title"));
        assert!(text.contains("Hi & bye <3"));
    }

    // ---- HTTP integration: execute_fetch against a one-shot mock server ----
    fn find_header_end(b: &[u8]) -> Option<usize> {
        b.windows(4).position(|w| w == b"\r\n\r\n")
    }

    async fn mock_http(body: String, ctype: &str) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/page");
        let ctype = ctype.to_string();
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
            // drain request body if present
            let header_end = find_header_end(&buf).unwrap_or(buf.len());
            let header_str = String::from_utf8_lossy(&buf[..header_end]);
            let clen = header_str
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                .and_then(|l| l.split(':').nth(1))
                .and_then(|s| s.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let body_start = header_end + 4;
            let mut have = buf.len().saturating_sub(body_start);
            while have < clen {
                let n = sock.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                have += n;
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n{}",
                ctype,
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.flush().await.unwrap();
        });
        (url, h)
    }

    #[tokio::test]
    async fn fetch_strips_html_over_http() {
        let html = String::from(
            "<html><body><script>evil()</script><h1>Rust Docs</h1><p>Use Vec for grow.</p></body></html>",
        );
        let (url, _h) = mock_http(html, "text/html; charset=utf-8").await;
        let cfg = crate::config::Config {
            fetch_allowlist: Vec::new(),
            fetch_timeout_secs: 10,
            fetch_max_bytes: 1 << 20,
            ..crate::config::Config::default()
        };
        let out = execute_fetch(&serde_json::json!({ "url": url }), &cfg).await;
        assert!(out.ok, "{}", out.output);
        assert!(out.output.contains("Rust Docs"));
        assert!(out.output.contains("Use Vec for grow."));
        assert!(!out.output.contains("evil()"));
        assert!(out.output.contains("text/html"));
    }

    #[tokio::test]
    async fn fetch_allowlist_denies_other_hosts() {
        let (_url, _h) = mock_http("hi".into(), "text/plain").await; // unused: denied before connect
        let cfg = crate::config::Config {
            fetch_allowlist: vec!["docs.rs".into(), "*.rust-lang.org".into()],
            ..crate::config::Config::default()
        };
        let out =
            execute_fetch(&serde_json::json!({ "url": "https://evil.example.com/x" }), &cfg).await;
        assert!(!out.ok);
        assert!(out.output.contains("not in the allowlist"));
    }

    #[tokio::test]
    async fn fetch_no_network_denies_without_allowlist() {
        // --no-network + empty allowlist: fetch must not bypass the egress block.
        let cfg = crate::config::Config {
            no_network: true,
            fetch_allowlist: Vec::new(),
            ..crate::config::Config::default()
        };
        let out =
            execute_fetch(&serde_json::json!({ "url": "https://example.com/x" }), &cfg).await;
        assert!(!out.ok);
        assert!(out.output.contains("--no-network"));
        assert!(out.output.contains("fetch_allowlist"));
    }

    #[tokio::test]
    async fn fetch_no_network_allows_when_allowlist_opts_in() {
        // --no-network + explicit allowlist: operator opted in, so an allowed
        // host is fetched (exercises the real HTTP path against the mock).
        let html = String::from("<html><body><p>docs ok</p></body></html>");
        let (url, _h) = mock_http(html, "text/html").await;
        // the mock is on 127.0.0.1; allow localhost so the opt-in path connects
        let cfg = crate::config::Config {
            no_network: true,
            fetch_allowlist: vec!["127.0.0.1".into(), "localhost".into()],
            fetch_timeout_secs: 10,
            fetch_max_bytes: 1 << 20,
            ..crate::config::Config::default()
        };
        let out = execute_fetch(&serde_json::json!({ "url": url }), &cfg).await;
        assert!(out.ok, "{}", out.output);
        assert!(out.output.contains("docs ok"));
    }
}
