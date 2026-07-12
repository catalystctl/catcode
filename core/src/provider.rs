// Multi-provider chat client. The internal conversation is always kept in
// OpenAI chat-completions shape (role:"tool", assistant `tool_calls`, ...)
// because every other layer (compaction, sanitization, subagents, session
// persistence) understands that shape. Translation to/from other wire
// protocols (Anthropic Messages API) happens only at the HTTP boundary,// driven by the active `ResolvedProvider`'s `kind`. Streams SSE chunks; emits
// delta/thinking/tool_call events. Retries on transient HTTP errors with
// exponential backoff (honors Retry-After).
use crate::config::{ProviderKind, ResolvedProvider};
use crate::logging::{estimate_tokens, TurnTimer};
use crate::message::{self, Message};
use crate::protocol::{emit, Event, ModelInfo};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

#[allow(dead_code)]
pub const DEFAULT_BASE_URL: &str = "https://api.code.umans.ai/v1";
const MODELS_INFO_PATH: &str = "/models/info";
/// Standard OpenAI `/models` list endpoint (first-party OpenAI + Gemini's
/// OpenAI-compatible shim). Used as a fallback when `/models/info` (Umans)
/// isn't served by the endpoint.
const OPENAI_MODELS_PATH: &str = "/models";
const CHAT_PATH: &str = "/chat/completions";
/// Anthropic Messages API requires an `anthropic-version` header.
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Anthropic endpoints: `{base_url}/messages` and `{base_url}/models`
/// (base_url conventionally ends in `/v1`, e.g. `https://api.anthropic.com/v1`).
const ANTHROPIC_MESSAGES_PATH: &str = "/messages";
const ANTHROPIC_MODELS_PATH: &str = "/models";

/// True if the base URL points at an Umans endpoint. Umans accepts extra
/// fields (reasoning_effort, reasoning_content replay) that vanilla OpenAI
/// servers reject with a 400 — gate those on this check.
pub fn is_umans(base_url: &str) -> bool {
    // Parse the HOST so a look-alike such as `https://api.umans.ai.evil.com/v1`
    // (host `api.umans.ai.evil.com`) is NOT mistaken for Umans. A naive
    // `contains("umans.ai")` substring match would enable Umans-only wire
    // fields (reasoning_effort / reasoning_content) on the wrong endpoint and
    // trigger 400s. Match `umans.ai` exactly or as a parent domain (subdomain).
    let host = base_url
        .split("://")
        .nth(1)
        .unwrap_or(base_url)
        .split(['/', '?'])
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    host == "umans.ai" || host.ends_with(".umans.ai")
}

/// Live account-wide concurrency usage from the Umans gateway's `/v1/usage`
/// endpoint. `used` = the number of concurrent sessions right now (across ALL
/// clients on this key, not just this process — the gateway tracks it), `limit`
/// = the plan's concurrency ceiling. `limit == None` means the plan has no
/// concurrency cap (unlimited) OR the field was absent — the footer renders
/// these as `∞`.
///
/// Returns `None` only when the HTTP request fails or the payload can't be
/// parsed — a successful fetch always yields `Some` (the inner fields may be
/// `None`). Polled every few seconds by the background task in `main.rs` so the
/// footer can show a live "Conc used/limit" ahead of tps; mirrors the
/// pi-provider-umans status widget.
pub struct UmansUsage {
    pub used: Option<u64>,
    pub limit: Option<u64>,
}

// ─── Provider-agnostic /usage command ────────────────────────────────────────
//
// `/usage` resolves the provider for the currently selected model and asks that
// provider for plan/window limits (5-hour, weekly, concurrency, …). Each
// first-party endpoint implements its own fetch+parse; unknown providers return
// `available: false` with a short explanation. The wire shape is a list of
// windows so the TUI can render one progress row per limit without knowing
// provider details.

/// One rate-limit / quota window for the `/usage` modal.
#[derive(Clone, Debug, Default)]
pub struct UsageWindow {
    /// Stable id for the window (e.g. `five_hour`, `weekly`, `concurrency`).
    pub id: String,
    /// Human label shown in the UI (e.g. "5-hour", "Weekly", "Concurrency").
    pub label: String,
    /// Used amount. For `unit == "percent"` this is utilization 0–100.
    pub used: Option<f64>,
    /// Limit amount. For `unit == "percent"` this is typically 100.
    pub limit: Option<f64>,
    /// How to interpret used/limit: `percent` | `sessions` | `requests` |
    /// `tokens` | `credits` | `count`.
    pub unit: String,
    /// Unix epoch seconds when this window resets (if known).
    pub resets_at: Option<i64>,
    /// Optional free-form detail (e.g. "resets in 42m").
    pub detail: Option<String>,
}

/// Provider-level usage snapshot returned by `fetch_provider_usage`.
#[derive(Clone, Debug, Default)]
pub struct ProviderUsage {
    /// False when this provider has no usage endpoint or the fetch failed.
    pub available: bool,
    /// Optional plan/subscription label (e.g. "Pro", "Max", "Team").
    pub plan: Option<String>,
    /// Explanation when unavailable, or a short note alongside windows.
    pub message: Option<String>,
    pub windows: Vec<UsageWindow>,
}

impl ProviderUsage {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            available: false,
            plan: None,
            message: Some(message.into()),
            windows: Vec::new(),
        }
    }

    /// Serialize for the `usage` event payload (without provider/model, which
    /// the command handler attaches).
    pub fn to_event_fields(&self) -> serde_json::Map<String, Value> {
        let mut m = serde_json::Map::new();
        m.insert("available".into(), json!(self.available));
        if let Some(ref p) = self.plan {
            m.insert("plan".into(), json!(p));
        }
        if let Some(ref msg) = self.message {
            m.insert("message".into(), json!(msg));
        }
        let windows: Vec<Value> = self
            .windows
            .iter()
            .map(|w| {
                let mut o = serde_json::Map::new();
                o.insert("id".into(), json!(w.id));
                o.insert("label".into(), json!(w.label));
                o.insert("unit".into(), json!(w.unit));
                if let Some(u) = w.used {
                    o.insert("used".into(), json!(u));
                }
                if let Some(l) = w.limit {
                    o.insert("limit".into(), json!(l));
                }
                if let Some(r) = w.resets_at {
                    o.insert("resets_at".into(), json!(r));
                }
                if let Some(ref d) = w.detail {
                    o.insert("detail".into(), json!(d));
                }
                Value::Object(o)
            })
            .collect();
        m.insert("windows".into(), Value::Array(windows));
        m
    }
}

/// Dispatch usage fetch for the resolved provider. Detects Umans / Codex /
/// Anthropic OAuth / xAI endpoints; everything else returns a clear
/// "not available" message so `/usage` never hard-errors.
pub async fn fetch_provider_usage(
    client: &reqwest::Client,
    rp: &ResolvedProvider,
) -> ProviderUsage {
    if is_umans(&rp.base_url) {
        return match rp.api_key.as_deref() {
            Some(k) => match fetch_umans_usage_full(client, &rp.base_url, k).await {
                Some(u) => u,
                None => ProviderUsage::unavailable(
                    "Could not reach Umans /usage — check your network and API key.",
                ),
            },
            None => ProviderUsage::unavailable("Umans is not authenticated — run /login."),
        };
    }
    if is_codex_endpoint(&rp.base_url) {
        return match rp.api_key.as_deref() {
            Some(k) => match fetch_codex_usage(client, &rp.base_url, k, &rp.headers).await {
                Some(u) => u,
                None => ProviderUsage::unavailable(
                    "Could not reach ChatGPT Codex usage — try /login again.",
                ),
            },
            None => ProviderUsage::unavailable("OpenAI Codex is not authenticated — run /login."),
        };
    }
    if rp.kind.is_anthropic() {
        // Claude subscription OAuth exposes 5h / weekly windows. API-key mode
        // does not have a comparable account-usage endpoint.
        if rp.oauth {
            return match rp.api_key.as_deref() {
                Some(k) => match fetch_anthropic_oauth_usage(client, k).await {
                    Some(u) => u,
                    None => ProviderUsage::unavailable(
                        "Could not reach Anthropic OAuth usage — try /login again.",
                    ),
                },
                None => ProviderUsage::unavailable("Anthropic is not authenticated — run /login."),
            };
        }
        return ProviderUsage::unavailable(
            "Anthropic API-key mode has no account usage endpoint. \
             Use a Claude Pro/Max subscription via /login for 5-hour and weekly limits, \
             or check console.anthropic.com for organization rate limits.",
        );
    }
    if is_gemini_endpoint(&rp.base_url) || is_code_assist_endpoint(&rp.base_url) {
        return ProviderUsage::unavailable(
            "Google Gemini does not expose plan usage via API. Check quotas in Google AI Studio / Cloud Console.",
        );
    }
    if is_xai_endpoint(&rp.base_url) || rp.name == "xai" {
        // SuperGrok / X Premium+ OAuth (same client Grok Build uses) can read
        // monthly credit usage from the cli-chat-proxy billing endpoint.
        return match rp.api_key.as_deref() {
            Some(k) => match fetch_xai_usage(client, k, &rp.headers).await {
                Some(u) => u,
                None => ProviderUsage::unavailable(
                    "Could not reach xAI billing — try /login again, or check console.x.ai.",
                ),
            },
            None => ProviderUsage::unavailable("xAI is not authenticated — run /login."),
        };
    }
    if is_opencode_go(&rp.base_url) {
        return ProviderUsage::unavailable(
            "OpenCode Go does not expose subscription usage via API yet.",
        );
    }
    // Last-chance probe: some OpenAI-compatible gateways mirror Umans' `/usage`.
    if let Some(k) = rp.api_key.as_deref() {
        if let Some(u) = fetch_umans_usage_full(client, &rp.base_url, k).await {
            if u.available && !u.windows.is_empty() {
                return u;
            }
        }
    }
    ProviderUsage::unavailable(format!(
        "Provider `{}` does not publish usage stats for /usage.",
        if rp.name.is_empty() {
            "unknown"
        } else {
            &rp.name
        }
    ))
}

/// Parse the Umans `/v1/usage` JSON payload into `UmansUsage`. Pure (no I/O) so
/// it can be unit-tested against the documented response shape:
/// `{ limits: { concurrency: { limit }, requests: { limit } },
///    usage: { requests_in_window, concurrent_sessions } }`.
/// `used` = `usage.concurrent_sessions`; `limit` = `limits.concurrency.limit`
/// (null/absent → `None`, rendered as ∞ by the UI).
pub fn parse_umans_usage(v: &Value) -> UmansUsage {
    let used = v
        .get("usage")
        .and_then(|u| u.get("concurrent_sessions"))
        .and_then(|c| c.as_u64());
    let limit = v
        .get("limits")
        .and_then(|l| l.get("concurrency"))
        .and_then(|c| c.get("limit"))
        .and_then(|l| l.as_u64());
    UmansUsage { used, limit }
}

/// Full Umans usage for the `/usage` modal: plan name, concurrency, requests
/// window, optional token totals, and reset countdown. Pure parser so tests
/// can cover the documented pi-provider-umans payload shape.
pub fn parse_umans_usage_full(v: &Value) -> ProviderUsage {
    let plan = v
        .get("plan")
        .and_then(|p| {
            p.get("display_name")
                .or_else(|| p.get("slug"))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.is_empty());

    let remaining_minutes = v
        .get("window")
        .and_then(|w| w.get("remaining_minutes"))
        .and_then(|m| m.as_f64().or_else(|| m.as_u64().map(|u| u as f64)));

    let reset_detail = remaining_minutes.map(|m| {
        if m < 1.0 {
            "resets soon".to_string()
        } else if m < 60.0 {
            format!("resets in {}m", m.round() as u64)
        } else {
            let h = (m / 60.0).floor() as u64;
            let mins = (m % 60.0).round() as u64;
            if mins == 0 {
                format!("resets in {h}h")
            } else {
                format!("resets in {h}h {mins}m")
            }
        }
    });

    let mut windows = Vec::new();

    let conc_used = v
        .get("usage")
        .and_then(|u| u.get("concurrent_sessions"))
        .and_then(|c| c.as_f64().or_else(|| c.as_u64().map(|u| u as f64)));
    let conc_limit = v
        .get("limits")
        .and_then(|l| l.get("concurrency"))
        .and_then(|c| c.get("limit"))
        .and_then(|l| l.as_f64().or_else(|| l.as_u64().map(|u| u as f64)));
    if conc_used.is_some() || conc_limit.is_some() {
        windows.push(UsageWindow {
            id: "concurrency".into(),
            label: "Concurrency".into(),
            used: conc_used,
            limit: conc_limit,
            unit: "sessions".into(),
            resets_at: None,
            detail: None,
        });
    }

    let req_used = v
        .get("usage")
        .and_then(|u| u.get("requests_in_window"))
        .and_then(|c| c.as_f64().or_else(|| c.as_u64().map(|u| u as f64)));
    let req_limit = v
        .get("limits")
        .and_then(|l| l.get("requests"))
        .and_then(|c| c.get("limit"))
        .and_then(|l| l.as_f64().or_else(|| l.as_u64().map(|u| u as f64)));
    if req_used.is_some() || req_limit.is_some() {
        windows.push(UsageWindow {
            id: "requests".into(),
            label: "Requests (window)".into(),
            used: req_used,
            limit: req_limit,
            unit: "requests".into(),
            resets_at: None,
            detail: reset_detail.clone(),
        });
    }

    let tokens_in = v
        .get("usage")
        .and_then(|u| u.get("tokens_in"))
        .and_then(|c| c.as_f64().or_else(|| c.as_u64().map(|u| u as f64)));
    let tokens_out = v
        .get("usage")
        .and_then(|u| u.get("tokens_out"))
        .and_then(|c| c.as_f64().or_else(|| c.as_u64().map(|u| u as f64)));
    if tokens_in.is_some() || tokens_out.is_some() {
        let tin = tokens_in.unwrap_or(0.0);
        let tout = tokens_out.unwrap_or(0.0);
        windows.push(UsageWindow {
            id: "tokens".into(),
            label: "Tokens (session window)".into(),
            used: Some(tin + tout),
            limit: None,
            unit: "tokens".into(),
            resets_at: None,
            detail: Some(format!(
                "in {} / out {}",
                format_token_count(tin),
                format_token_count(tout)
            )),
        });
    }

    if windows.is_empty() {
        return ProviderUsage::unavailable("Umans returned no usage fields.");
    }
    ProviderUsage {
        available: true,
        plan,
        message: None,
        windows,
    }
}

fn format_token_count(n: f64) -> String {
    if n >= 1_000_000.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.1}k", n / 1_000.0)
    } else {
        format!("{}", n.round() as u64)
    }
}

pub async fn fetch_umans_usage(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Option<UmansUsage> {
    let v = fetch_umans_usage_json(client, base_url, api_key).await?;
    Some(parse_umans_usage(&v))
}

async fn fetch_umans_usage_full(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Option<ProviderUsage> {
    let v = fetch_umans_usage_json(client, base_url, api_key).await?;
    Some(parse_umans_usage_full(&v))
}

async fn fetch_umans_usage_json(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Option<Value> {
    // base_url conventionally ends in `/v1` (e.g. https://api.code.umans.ai/v1),
    // so the usage endpoint is `{base_url}/usage` — matching how the chat path
    // is built as `{base_url}/chat/completions`. (The pi-provider-umans build
    // is `{base-without-v1}/v1/usage`; both resolve to the same URL.)
    let url = format!("{}/usage", base_url.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .bearer_auth(api_key)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json().await.ok()
}

/// Label a Codex/ChatGPT rate-limit window from its duration in seconds.
/// Primary is typically 5h (18000); secondary is weekly (604800).
fn window_label_from_seconds(secs: u64, fallback: &str) -> (String, String) {
    // Allow small drift around the documented windows.
    if (17_000..=19_000).contains(&secs) {
        return ("five_hour".into(), "5-hour".into());
    }
    if (3 * 3600 - 300..=3 * 3600 + 300).contains(&secs) {
        return ("three_hour".into(), "3-hour".into());
    }
    if (6 * 24 * 3600..=8 * 24 * 3600).contains(&secs) {
        return ("weekly".into(), "Weekly".into());
    }
    if secs >= 3600 {
        let h = secs / 3600;
        return (format!("{h}h"), format!("{h}-hour"));
    }
    if secs >= 60 {
        let m = secs / 60;
        return (format!("{m}m"), format!("{m}-minute"));
    }
    (fallback.into(), fallback.into())
}

/// Parse ChatGPT Codex `/wham/usage` JSON into provider usage windows.
/// Shape (simplified): `{ plan_type, rate_limit: { primary_window, secondary_window },
/// credits?: { balance, has_credits, unlimited } }` where each window has
/// `used_percent`, `limit_window_seconds`, `reset_at`.
pub fn parse_codex_usage(v: &Value) -> ProviderUsage {
    let plan = v
        .get("plan_type")
        .and_then(|p| p.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            // Sometimes nested under plan
            v.get("plan").and_then(|p| {
                p.as_str().map(|s| s.to_string()).or_else(|| {
                    p.get("type")
                        .or_else(|| p.get("name"))
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string())
                })
            })
        });

    let mut windows = Vec::new();

    let rate_limit = v.get("rate_limit").and_then(|r| {
        // Payload may double-wrap Option: null, object, or nested.
        if r.is_null() {
            None
        } else {
            Some(r)
        }
    });

    if let Some(rl) = rate_limit {
        for (key, fallback_id, fallback_label) in [
            ("primary_window", "primary", "Primary"),
            ("secondary_window", "secondary", "Secondary"),
        ] {
            if let Some(w) = parse_codex_rate_window(rl.get(key), fallback_id, fallback_label) {
                windows.push(w);
            }
        }
    }

    // Credits balance (optional).
    if let Some(credits) = v.get("credits").filter(|c| !c.is_null()) {
        let unlimited = credits
            .get("unlimited")
            .and_then(|u| u.as_bool())
            .unwrap_or(false);
        let balance = credits.get("balance").and_then(|b| {
            b.as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| b.as_f64())
                .or_else(|| b.as_u64().map(|u| u as f64))
        });
        if unlimited {
            windows.push(UsageWindow {
                id: "credits".into(),
                label: "Credits".into(),
                used: None,
                limit: None,
                unit: "credits".into(),
                resets_at: None,
                detail: Some("unlimited".into()),
            });
        } else if let Some(bal) = balance {
            windows.push(UsageWindow {
                id: "credits".into(),
                label: "Credits remaining".into(),
                used: None,
                limit: Some(bal),
                unit: "credits".into(),
                resets_at: None,
                detail: Some(format!("{bal}")),
            });
        }
    }

    if windows.is_empty() {
        return ProviderUsage::unavailable("Codex returned no rate-limit windows.");
    }
    ProviderUsage {
        available: true,
        plan,
        message: None,
        windows,
    }
}

fn parse_codex_rate_window(
    node: Option<&Value>,
    fallback_id: &str,
    fallback_label: &str,
) -> Option<UsageWindow> {
    let w = node?;
    // Handle optional double-boxing / null.
    let w = if w.is_null() {
        return None;
    } else {
        w
    };
    let used_percent = w
        .get("used_percent")
        .and_then(|p| p.as_f64().or_else(|| p.as_u64().map(|u| u as f64)))?;
    let window_secs = w
        .get("limit_window_seconds")
        .and_then(|s| s.as_u64().or_else(|| s.as_f64().map(|f| f as u64)))
        .unwrap_or(0);
    let resets_at = w
        .get("reset_at")
        .and_then(|r| r.as_i64().or_else(|| r.as_u64().map(|u| u as i64)));
    let (id, label) = if window_secs > 0 {
        window_label_from_seconds(window_secs, fallback_id)
    } else {
        (fallback_id.into(), fallback_label.into())
    };
    let detail = resets_at.and_then(format_resets_at_detail);
    Some(UsageWindow {
        id,
        label,
        used: Some(used_percent),
        limit: Some(100.0),
        unit: "percent".into(),
        resets_at,
        detail,
    })
}

fn format_resets_at_detail(resets_at: i64) -> Option<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    let delta = resets_at - now;
    if delta <= 0 {
        return Some("resets soon".into());
    }
    let mins = delta / 60;
    if mins < 60 {
        return Some(format!("resets in {mins}m"));
    }
    let hours = mins / 60;
    let rem_m = mins % 60;
    if hours < 48 {
        if rem_m == 0 {
            Some(format!("resets in {hours}h"))
        } else {
            Some(format!("resets in {hours}h {rem_m}m"))
        }
    } else {
        let days = hours / 24;
        let rem_h = hours % 24;
        if rem_h == 0 {
            Some(format!("resets in {days}d"))
        } else {
            Some(format!("resets in {days}d {rem_h}h"))
        }
    }
}

async fn fetch_codex_usage(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    headers: &[(String, String)],
) -> Option<ProviderUsage> {
    // Chat path is `{chatgpt.com/backend-api}/codex/...`; usage lives at
    // `{chatgpt.com/backend-api}/wham/usage` (official codex CLI).
    let trimmed = base_url.trim_end_matches('/');
    let backend = trimmed.strip_suffix("/codex").unwrap_or(trimmed);
    let url = format!("{backend}/wham/usage");
    let mut req = client
        .get(&url)
        .bearer_auth(api_key)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(10));
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    Some(parse_codex_usage(&v))
}

/// Parse Anthropic OAuth `/api/oauth/usage` utilization payload.
/// Shape: `{ five_hour?: { utilization, resets_at }, seven_day?: {...},
/// seven_day_opus?: {...}, seven_day_sonnet?: {...}, extra_usage?: {...} }`.
/// `utilization` is 0–100; `resets_at` is ISO-8601 or unix seconds.
pub fn parse_anthropic_oauth_usage(v: &Value) -> ProviderUsage {
    let mut windows = Vec::new();
    for (key, label) in [
        ("five_hour", "5-hour"),
        ("seven_day", "Weekly"),
        ("seven_day_opus", "Weekly (Opus)"),
        ("seven_day_sonnet", "Weekly (Sonnet)"),
        ("seven_day_oauth_apps", "Weekly (OAuth apps)"),
    ] {
        if let Some(w) = parse_anthropic_rate_limit(v.get(key), key, label) {
            windows.push(w);
        }
    }
    // Extra usage / overage credits.
    if let Some(extra) = v.get("extra_usage").filter(|e| !e.is_null()) {
        let enabled = extra
            .get("is_enabled")
            .and_then(|b| b.as_bool())
            .unwrap_or(true);
        if enabled {
            let util = extra
                .get("utilization")
                .and_then(|u| u.as_f64().or_else(|| u.as_u64().map(|n| n as f64)));
            let used_credits = extra
                .get("used_credits")
                .and_then(|u| u.as_f64().or_else(|| u.as_u64().map(|n| n as f64)));
            let monthly = extra
                .get("monthly_limit")
                .and_then(|u| u.as_f64().or_else(|| u.as_u64().map(|n| n as f64)));
            if util.is_some() || used_credits.is_some() || monthly.is_some() {
                windows.push(UsageWindow {
                    id: "extra_usage".into(),
                    label: "Extra usage".into(),
                    used: util.or(used_credits),
                    limit: if util.is_some() { Some(100.0) } else { monthly },
                    unit: if util.is_some() {
                        "percent".into()
                    } else {
                        "credits".into()
                    },
                    resets_at: None,
                    detail: match (used_credits, monthly) {
                        (Some(u), Some(m)) => Some(format!("{u}/{m} credits")),
                        _ => None,
                    },
                });
            }
        }
    }

    if windows.is_empty() {
        return ProviderUsage::unavailable(
            "No Claude subscription limits returned (API-key accounts have no 5h/weekly windows).",
        );
    }
    ProviderUsage {
        available: true,
        plan: None,
        message: None,
        windows,
    }
}

fn parse_anthropic_rate_limit(node: Option<&Value>, id: &str, label: &str) -> Option<UsageWindow> {
    let w = node.filter(|n| !n.is_null())?;
    let utilization = w
        .get("utilization")
        .and_then(|u| u.as_f64().or_else(|| u.as_u64().map(|n| n as f64)))?;
    let resets_at = w.get("resets_at").and_then(|r| {
        if let Some(n) = r.as_i64().or_else(|| r.as_u64().map(|u| u as i64)) {
            return Some(n);
        }
        // ISO-8601 string → unix seconds (best-effort).
        r.as_str().and_then(parse_iso8601_unix)
    });
    Some(UsageWindow {
        id: id.into(),
        label: label.into(),
        used: Some(utilization),
        limit: Some(100.0),
        unit: "percent".into(),
        resets_at,
        detail: resets_at.and_then(format_resets_at_detail),
    })
}

/// Best-effort ISO-8601 → unix seconds. Handles `2026-07-09T12:00:00Z` and
/// bare unix digit strings. Offset forms are treated as UTC (close enough for
/// "resets in …" display).
fn parse_iso8601_unix(s: &str) -> Option<i64> {
    // Format: YYYY-MM-DDTHH:MM:SS[.frac][Z|±HH:MM]
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // If it's pure digits, treat as unix already.
    if let Ok(n) = s.parse::<i64>() {
        return Some(n);
    }
    // Character positions: 0..4 year, 5..7 month, 8..10 day, 11..13 hour,
    // 14..16 min, 17..19 sec. Fractional seconds and timezone are ignored.
    if s.len() < 19 {
        return None;
    }
    let bytes = s.as_bytes();
    let year: i64 = std::str::from_utf8(&bytes[0..4]).ok()?.parse().ok()?;
    let month: i64 = std::str::from_utf8(&bytes[5..7]).ok()?.parse().ok()?;
    let day: i64 = std::str::from_utf8(&bytes[8..10]).ok()?.parse().ok()?;
    let hour: i64 = std::str::from_utf8(&bytes[11..13]).ok()?.parse().ok()?;
    let min: i64 = std::str::from_utf8(&bytes[14..16]).ok()?.parse().ok()?;
    let sec: i64 = std::str::from_utf8(&bytes[17..19]).ok()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days from civil date (Howard Hinnant algorithm) → unix.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}

async fn fetch_anthropic_oauth_usage(
    client: &reqwest::Client,
    access_token: &str,
) -> Option<ProviderUsage> {
    // Same endpoint Claude Code's /usage uses (api.anthropic.com OAuth usage).
    let url = "https://api.anthropic.com/api/oauth/usage";
    let resp = client
        .get(url)
        .bearer_auth(access_token)
        .header("anthropic-beta", crate::oauth::CLAUDE_OAUTH_BETA)
        .header("user-agent", crate::oauth::CLAUDE_OAUTH_USER_AGENT)
        .header("x-app", crate::oauth::CLAUDE_OAUTH_X_APP)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    Some(parse_anthropic_oauth_usage(&v))
}

// ─── xAI / SuperGrok weekly usage ────────────────────────────────────────────
//
// SuperGrok uses one shared **weekly** usage pool across Chat / Imagine /
// Voice / Build / API (docs.x.ai/grok/faq#usage--limits). Grok Build's
// `/usage` reads it from:
//   GET https://cli-chat-proxy.grok.com/v1/billing?format=credits
// Shape (the authoritative SuperGrok view — matches the website Usage tab):
//   { config: {
//       creditUsagePercent: 30.0,                 // total weekly % used
//       currentPeriod: { type: "USAGE_PERIOD_TYPE_WEEKLY", start, end },
//       productUsage: [{ product: "GrokBuild", usagePercent: 29.0 }, …],
//       onDemandCap/Used, prepaidBalance, …
//   } }
// Without `?format=credits` the same host returns a legacy raw-credit shape
// (`monthlyLimit`/`used`) that does NOT match the website % — we always prefer
// the credits format. Optional plan enrichment from grok.com/rest/subscriptions.

/// SuperGrok weekly-pool endpoint (Grok Build / website-compatible).
const XAI_BILLING_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";
/// Consumer subscription status (tier name) for SuperGrok / Premium+ accounts.
const XAI_SUBSCRIPTIONS_URL: &str = "https://grok.com/rest/subscriptions";

/// Unwrap Grok's `{ "val": number }` money/credit wrappers (or a bare number).
fn xai_val_number(v: Option<&Value>) -> Option<f64> {
    let v = v?;
    if let Some(n) = v.as_f64().or_else(|| v.as_u64().map(|u| u as f64)) {
        return Some(n);
    }
    if let Some(n) = v
        .get("val")
        .and_then(|x| x.as_f64().or_else(|| x.as_u64().map(|u| u as f64)))
    {
        return Some(n);
    }
    // Some payloads stringify the value.
    if let Some(s) = v.as_str().or_else(|| v.get("val").and_then(|x| x.as_str())) {
        return s.parse::<f64>().ok();
    }
    None
}

/// Map SuperGrok subscription tier enum to a short plan label.
fn xai_tier_label(tier: &str) -> String {
    match tier {
        "SUBSCRIPTION_TIER_GROK_PRO" | "SUBSCRIPTION_TIER_SUPERGROK" => "SuperGrok".into(),
        "SUBSCRIPTION_TIER_GROK_HEAVY" | "SUBSCRIPTION_TIER_SUPERGROK_HEAVY" => {
            "SuperGrok Heavy".into()
        }
        "SUBSCRIPTION_TIER_GROK_LITE" | "SUBSCRIPTION_TIER_SUPERGROK_LITE" => {
            "SuperGrok Lite".into()
        }
        "SUBSCRIPTION_TIER_X_PREMIUM_PLUS" => "X Premium+".into(),
        "SUBSCRIPTION_TIER_X_PREMIUM" => "X Premium".into(),
        other => {
            // Strip common prefixes for unknown tiers.
            other
                .trim_start_matches("SUBSCRIPTION_TIER_")
                .replace('_', " ")
                .to_string()
        }
    }
}

/// Friendly product names for SuperGrok productUsage breakdown.
fn xai_product_label(product: &str) -> String {
    match product {
        "GrokBuild" | "Build" => "Build".into(),
        "Api" | "API" => "API".into(),
        "Chat" => "Chat".into(),
        "Imagine" => "Imagine".into(),
        "Voice" => "Voice".into(),
        other => other.to_string(),
    }
}

/// Parse SuperGrok `GET /v1/billing?format=credits` (website-matching %) into
/// provider usage windows. Falls back to the legacy raw-credit shape when the
/// credits-format fields are absent.
pub fn parse_xai_billing(v: &Value) -> ProviderUsage {
    // Payload may be `{ config: {...} }` or the config object itself.
    let cfg = v.get("config").unwrap_or(v);

    // ── Preferred: format=credits (matches grok.com Settings → Usage) ──
    let credit_pct = cfg
        .get("creditUsagePercent")
        .and_then(|p| p.as_f64().or_else(|| p.as_u64().map(|u| u as f64)));

    // Weekly window from currentPeriod (type USAGE_PERIOD_TYPE_WEEKLY).
    let period = cfg.get("currentPeriod");
    let period_end = period
        .and_then(|p| p.get("end"))
        .and_then(|s| s.as_str())
        .and_then(parse_iso8601_unix)
        .or_else(|| {
            cfg.get("billingPeriodEnd")
                .and_then(|s| s.as_str())
                .and_then(parse_iso8601_unix)
        });
    let period_start = period
        .and_then(|p| p.get("start"))
        .and_then(|s| s.as_str())
        .and_then(parse_iso8601_unix)
        .or_else(|| {
            cfg.get("billingPeriodStart")
                .and_then(|s| s.as_str())
                .and_then(parse_iso8601_unix)
        });
    let _ = period_start; // reserved for future "window elapsed" UI
    let period_type = period
        .and_then(|p| p.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    let is_weekly = period_type.contains("WEEKLY") || period_type.is_empty();

    let mut windows = Vec::new();

    if let Some(pct) = credit_pct {
        let label = if is_weekly {
            "Weekly usage".to_string()
        } else {
            "Usage".to_string()
        };
        windows.push(UsageWindow {
            id: if is_weekly {
                "weekly".into()
            } else {
                "usage".into()
            },
            label,
            used: Some(pct),
            limit: Some(100.0),
            unit: "percent".into(),
            resets_at: period_end,
            detail: period_end.and_then(format_resets_at_detail),
        });

        // Per-product breakdown (API, Build, Chat, Imagine, Voice) — same as
        // the website Usage tab. Percentages are shares of the weekly pool.
        if let Some(arr) = cfg.get("productUsage").and_then(|a| a.as_array()) {
            for entry in arr {
                let product = entry
                    .get("product")
                    .and_then(|p| p.as_str())
                    .unwrap_or("other");
                let p_pct = entry
                    .get("usagePercent")
                    .and_then(|p| p.as_f64().or_else(|| p.as_u64().map(|u| u as f64)));
                let Some(p_pct) = p_pct else { continue };
                // Skip zero-share products to keep the modal clean.
                if p_pct <= 0.0 {
                    continue;
                }
                windows.push(UsageWindow {
                    id: format!("product_{}", product.to_ascii_lowercase()),
                    label: xai_product_label(product),
                    used: Some(p_pct),
                    limit: Some(100.0),
                    unit: "percent".into(),
                    resets_at: None,
                    detail: None,
                });
            }
        }
    } else {
        // ── Legacy fallback: raw used/monthlyLimit (does NOT match website %) ──
        let used = xai_val_number(cfg.get("used"));
        let limit = xai_val_number(cfg.get("monthlyLimit"))
            .or_else(|| xai_val_number(cfg.get("weeklyLimit")))
            .or_else(|| xai_val_number(cfg.get("limit")));
        if used.is_some() || limit.is_some() {
            windows.push(UsageWindow {
                id: "weekly".into(),
                label: "Weekly usage".into(),
                used,
                limit,
                unit: "credits".into(),
                resets_at: period_end,
                detail: period_end.and_then(format_resets_at_detail),
            });
        }
    }

    // On-demand / prepaid (extra usage credits).
    let on_demand_cap = xai_val_number(cfg.get("onDemandCap"));
    let on_demand_used =
        xai_val_number(cfg.get("onDemandUsed")).or_else(|| xai_val_number(cfg.get("onDemand")));
    let prepaid = xai_val_number(cfg.get("prepaidBalance"));

    if let Some(cap) = on_demand_cap.filter(|c| *c > 0.0) {
        windows.push(UsageWindow {
            id: "on_demand".into(),
            label: "On-demand cap".into(),
            used: on_demand_used,
            limit: Some(cap),
            unit: "credits".into(),
            resets_at: None,
            detail: None,
        });
    } else if let Some(od) = on_demand_used.filter(|c| *c > 0.0) {
        windows.push(UsageWindow {
            id: "on_demand".into(),
            label: "On-demand used".into(),
            used: Some(od),
            limit: None,
            unit: "credits".into(),
            resets_at: None,
            detail: None,
        });
    }
    if let Some(bal) = prepaid.filter(|b| *b > 0.0) {
        windows.push(UsageWindow {
            id: "prepaid".into(),
            label: "Extra credits".into(),
            used: None,
            limit: Some(bal),
            unit: "credits".into(),
            resets_at: None,
            detail: Some(format!("{} remaining", format_token_count(bal))),
        });
    }

    if windows.is_empty() {
        return ProviderUsage::unavailable("xAI billing returned no credit fields.");
    }

    // Plan label: not always present on the billing payload; subscriptions
    // enrichment fills this in when available.
    let plan = cfg
        .get("plan")
        .and_then(|p| p.as_str())
        .or_else(|| cfg.get("tier").and_then(|t| t.as_str()))
        .or_else(|| cfg.get("subscription_tier").and_then(|t| t.as_str()))
        .or_else(|| v.get("plan").and_then(|p| p.as_str()))
        .map(|s| {
            if s.starts_with("SUBSCRIPTION_TIER_") {
                xai_tier_label(s)
            } else {
                s.to_string()
            }
        });

    ProviderUsage {
        available: true,
        plan,
        message: None,
        windows,
    }
}

/// Pull SuperGrok / Premium tier from grok.com subscriptions (best-effort).
fn parse_xai_subscription_plan(v: &Value) -> Option<String> {
    let subs = v.get("subscriptions")?.as_array()?;
    // Prefer an active subscription; otherwise take the first.
    let active = subs.iter().find(|s| {
        s.get("status")
            .and_then(|st| st.as_str())
            .is_some_and(|st| st.contains("ACTIVE"))
    });
    let sub = active.or_else(|| subs.first())?;
    let tier = sub.get("tier").and_then(|t| t.as_str())?;
    Some(xai_tier_label(tier))
}

async fn fetch_xai_subscription_plan(
    client: &reqwest::Client,
    access_token: &str,
) -> Option<String> {
    let resp = client
        .get(XAI_SUBSCRIPTIONS_URL)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    parse_xai_subscription_plan(&v)
}

async fn fetch_xai_usage(
    client: &reqwest::Client,
    access_token: &str,
    headers: &[(String, String)],
) -> Option<ProviderUsage> {
    let mut req = client
        .get(XAI_BILLING_URL)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(10));
    for (k, v) in headers {
        // Don't override Authorization / Accept we just set.
        let kl = k.to_ascii_lowercase();
        if kl == "authorization" || kl == "accept" {
            continue;
        }
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    let mut usage = parse_xai_billing(&v);
    // Enrich plan label from consumer subscriptions when billing omits it.
    if usage.plan.is_none() {
        if let Some(plan) = fetch_xai_subscription_plan(client, access_token).await {
            usage.plan = Some(plan);
        }
    }
    Some(usage)
}

/// The reasoning levels offered when a model advertises none of its own
/// (and as the fallback set the TUI cycles through).
pub const DEFAULT_THINKING_LEVELS: &[&str] = &["low", "medium", "high"];

/// Resolve a requested reasoning effort against a model's advertised thinking
/// levels. If the model declares no levels (empty slice) the request passes
/// through unchanged. If it does, an unsupported effort is clamped to the
/// closest preferred level (high → medium → low → … → first listed) so the
/// model never receives an effort it can't handle (e.g. GLM only takes "high").
/// Comparison is case-insensitive; the returned string preserves the model's
/// own casing so the wire field matches what the endpoint expects.
pub fn resolve_effort(requested: &str, levels: &[String]) -> String {
    if levels.is_empty() {
        return requested.to_string();
    }
    if let Some(hit) = levels.iter().find(|l| l.eq_ignore_ascii_case(requested)) {
        return hit.clone();
    }
    for pref in ["high", "medium", "low", "minimal", "none"] {
        if let Some(hit) = levels.iter().find(|l| l.eq_ignore_ascii_case(pref)) {
            return hit.clone();
        }
    }
    levels[0].clone()
}

/// Hard cap on a single summarize request's user payload. Larger middles are
/// map-reduced in chunks so the summarize call itself never blows the model
/// context (which used to make compaction fall back to an empty drop marker).
const MAX_SUMMARY_INPUT_CHARS: usize = 100_000;
/// Per-tool-result char budget inside the summarize payload (after digesting
/// oversized results). Keeps path/command signal without re-sending 48KB dumps.
const SUMMARY_TOOL_RESULT_CHARS: usize = 1_500;
/// Max tokens for the combined summary+facts reply.
const SUMMARY_MAX_TOKENS: u32 = 3072;

/// Truncate `s` at a char boundary, appending an ellipsis when cut.
fn trunc_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n).collect();
    out.push('…');
    out
}

/// Build a compact, image-stripped string of a message for the summarization
/// prompt. Re-serializing a multimodal message verbatim would POST megabytes
/// of base64 image data to the model (costly, and it can blow the summary
/// request's own context); image parts are replaced with a short placeholder.
/// Oversized tool results and write/edit payloads are truncated so a tool-heavy
/// middle can still be summarized instead of failing the HTTP call.
fn message_for_summary(m: &Message) -> String {
    let v: Value = m.into();
    let mut clean = v;
    if let Some(arr) = clean.get_mut("content").and_then(|v| v.as_array_mut()) {
        for part in arr.iter_mut() {
            if part.get("type").and_then(|v| v.as_str()) == Some("image_url") {
                *part = json!({ "type": "text", "text": "[image omitted in summary]" });
            }
        }
    }
    // Truncate large tool-result content strings.
    if clean.get("role").and_then(|r| r.as_str()) == Some("tool") {
        if let Some(c) = clean.get("content").and_then(|c| c.as_str()) {
            if c.len() > SUMMARY_TOOL_RESULT_CHARS {
                let head = trunc_chars(c, SUMMARY_TOOL_RESULT_CHARS / 2);
                let tail = {
                    let chars: Vec<char> = c.chars().collect();
                    let n = SUMMARY_TOOL_RESULT_CHARS / 2;
                    if chars.len() > n {
                        chars[chars.len() - n..].iter().collect::<String>()
                    } else {
                        String::new()
                    }
                };
                clean["content"] = json!(format!(
                    "{head}\n…[truncated {} chars for summary]…\n{tail}",
                    c.len()
                ));
            }
        }
    }
    // Truncate huge tool-call argument payloads (write_file content, etc.).
    if let Some(calls) = clean.get_mut("tool_calls").and_then(|v| v.as_array_mut()) {
        for tc in calls.iter_mut() {
            if let Some(args) = tc
                .pointer_mut("/function/arguments")
                .and_then(|a| a.as_str().map(|s| s.to_string()))
            {
                if args.len() > SUMMARY_TOOL_RESULT_CHARS {
                    *tc.pointer_mut("/function/arguments").unwrap() =
                        json!(trunc_chars(&args, SUMMARY_TOOL_RESULT_CHARS));
                }
            }
        }
    }
    serde_json::to_string(&clean).unwrap_or_default()
}

/// Serialize messages for a summarize call, then split into char-budgeted chunks
/// so each HTTP request stays under `MAX_SUMMARY_INPUT_CHARS`.
fn summary_payload_chunks(messages: &[Message]) -> Vec<String> {
    let parts: Vec<String> = messages.iter().map(message_for_summary).collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for p in parts {
        if !cur.is_empty() && cur.len() + 1 + p.len() > MAX_SUMMARY_INPUT_CHARS {
            chunks.push(std::mem::take(&mut cur));
        }
        if p.len() > MAX_SUMMARY_INPUT_CHARS {
            // A single message still oversized after truncation — hard-slice it.
            let mut offset = 0;
            let bytes = p.as_bytes();
            while offset < bytes.len() {
                let mut end = (offset + MAX_SUMMARY_INPUT_CHARS).min(bytes.len());
                while end > offset && !p.is_char_boundary(end) {
                    end -= 1;
                }
                if end == offset {
                    break;
                }
                chunks.push(p[offset..end].to_string());
                offset = end;
            }
            continue;
        }
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(&p);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    chunks
}

fn summary_system_prompt(instructions: Option<&str>) -> String {
    const BASE_SYS: &str = "Summarize the following conversation turns in structured format. Preserve: decisions made, file paths touched, the user's goal, and any unresolved errors.\n\nAlso extract durable project facts worth remembering across future sessions (conventions, structure, key decisions, gotchas). If none, put the single word none under <facts>.\n\nUse this exact format:\n<summary>\n 1. Primary Request and Intent\n 2. Key Technical Concepts\n 3. Files and Code Sections\n 4. Errors and Fixes\n 5. Problem Solving\n 6. All User Messages\n 7. Pending Tasks\n 8. Current Work\n 9. Optional Next Step\n</summary>\n<facts>\n- fact one\n- fact two\n</facts>";
    match instructions.map(str::trim).filter(|s| !s.is_empty()) {
        Some(extra) => format!(
            "{BASE_SYS}\n\nThe user provided the following guidance for what to preserve in this summary — honor it above the default priorities:\n{extra}"
        ),
        None => BASE_SYS.to_string(),
    }
}

/// Parse a combined summarize+facts reply into `(summary, optional_facts)`.
fn parse_summary_and_facts(raw: &str) -> (String, Option<String>) {
    let trimmed = raw.trim();
    let facts = {
        let lower = trimmed.to_ascii_lowercase();
        if let Some(start) = lower.find("<facts>") {
            let after = start + "<facts>".len();
            let end = lower[after..]
                .find("</facts>")
                .map(|i| after + i)
                .unwrap_or(trimmed.len());
            let body = trimmed[after..end].trim();
            if body.is_empty() || body.eq_ignore_ascii_case("none") {
                None
            } else {
                Some(body.to_string())
            }
        } else {
            None
        }
    };
    let summary = {
        let lower = trimmed.to_ascii_lowercase();
        if let Some(start) = lower.find("<summary>") {
            let after = start + "<summary>".len();
            let end = lower[after..]
                .find("</summary>")
                .map(|i| after + i)
                .unwrap_or_else(|| {
                    lower[after..]
                        .find("<facts>")
                        .map(|i| after + i)
                        .unwrap_or(trimmed.len())
                });
            trimmed[after..end].trim().to_string()
        } else if let Some(facts_at) = lower.find("<facts>") {
            trimmed[..facts_at].trim().to_string()
        } else {
            trimmed.to_string()
        }
    };
    (summary, facts)
}

/// Summarize a slice of messages into one system message. Used by context
/// compaction so dropped turns become a short recap instead of vanishing.
/// Non-streaming, cheap; returns None on any failure (caller keeps the
/// naive drop-oldest fallback). Protocol-agnostic: branches on the provider's
/// `kind` (OpenAI chat-completions vs Anthropic Messages).
///
/// Oversized middles are truncated per-message and map-reduced in chunks so the
/// summarize HTTP call itself rarely fails from context overflow.
#[allow(dead_code)] // convenience wrapper: production uses summarize_and_extract;
                    // retained as API + exercised by the mock tests below
pub async fn summarize(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
    model: &str,
    messages: &[Message],
    cancel: &CancellationToken,
    instructions: Option<&str>,
) -> Option<String> {
    summarize_and_extract(client, provider, model, messages, cancel, instructions)
        .await
        .map(|(s, _)| s)
}

/// One-shot summarize + durable-fact extraction (single model call). Returns
/// `(summary, facts)` where facts is `None` when the model reported nothing
/// durable. Prefer this over separate `summarize` + `extract_facts` calls.
pub async fn summarize_and_extract(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
    model: &str,
    messages: &[Message],
    cancel: &CancellationToken,
    instructions: Option<&str>,
) -> Option<(String, Option<String>)> {
    let sys = summary_system_prompt(instructions);
    let chunks = summary_payload_chunks(messages);
    if chunks.len() == 1 {
        let raw = complete_text(
            client,
            provider,
            model,
            &sys,
            &chunks[0],
            SUMMARY_MAX_TOKENS,
            cancel,
        )
        .await?;
        return Some(parse_summary_and_facts(&raw));
    }
    // Map-reduce: summarize each chunk, then merge.
    let mut partials: Vec<String> = Vec::with_capacity(chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        let chunk_sys = format!(
            "{sys}\n\nThis is partial chunk {} of {}. Summarize only this chunk; a later merge will combine them.",
            i + 1,
            chunks.len()
        );
        let part = complete_text(
            client,
            provider,
            model,
            &chunk_sys,
            chunk,
            SUMMARY_MAX_TOKENS,
            cancel,
        )
        .await?;
        partials.push(part);
    }
    let merge_user = {
        let joined = partials.join("\n\n---\n\n");
        if joined.len() <= MAX_SUMMARY_INPUT_CHARS {
            joined
        } else {
            // Hierarchical reduce would be nicer; hard-cap keeps the merge call
            // from itself blowing the model context (which used to make compact
            // fall back to an empty drop marker).
            let mut out = String::new();
            for p in &partials {
                if out.len() + p.len() + 8 > MAX_SUMMARY_INPUT_CHARS {
                    break;
                }
                if !out.is_empty() {
                    out.push_str("\n\n---\n\n");
                }
                out.push_str(p);
            }
            if out.is_empty() {
                trunc_chars(&joined, MAX_SUMMARY_INPUT_CHARS)
            } else {
                out
            }
        }
    };
    let merge_sys = format!(
        "{sys}\n\nBelow are partial summaries of earlier conversation chunks. Merge them into one final <summary> and one <facts> block. Deduplicate; prefer later info when they conflict."
    );
    let raw = complete_text(
        client,
        provider,
        model,
        &merge_sys,
        &merge_user,
        SUMMARY_MAX_TOKENS,
        cancel,
    )
    .await?;
    Some(parse_summary_and_facts(&raw))
}

/// Extract durable facts worth remembering across future sessions from a slice of
/// the conversation. Best-effort (returns None on any failure, or if there is
/// nothing durable). Used by the session memory extraction hook on compaction.
/// Prefer [`summarize_and_extract`] when a summary is also needed (one call).
/// Protocol-agnostic: branches on the provider's `kind`.
#[allow(dead_code)] // convenience wrapper over summarize_and_extract; kept for API + tests
pub async fn extract_facts(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
    model: &str,
    messages: &[Message],
    cancel: &CancellationToken,
) -> Option<String> {
    summarize_and_extract(client, provider, model, messages, cancel, None)
        .await
        .and_then(|(_, facts)| facts)
}

/// One-shot text completion (no tools, no streaming). Returns the model's text
/// reply. Branches on provider kind so callers (summarize/extract_facts) stay
/// protocol-agnostic. `max_tokens` caps the reply (Anthropic requires it;
/// OpenAI servers ignore/apply it tolerantly).
async fn complete_text(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
    model: &str,
    system: &str,
    user: &str,
    max_tokens: u32,
    cancel: &CancellationToken,
) -> Option<String> {
    match provider.kind {
        ProviderKind::OpenAI => {
            openai_complete(client, provider, model, system, user, max_tokens, cancel).await
        }
        ProviderKind::Anthropic => {
            anthropic_complete(client, provider, model, system, user, max_tokens, cancel).await
        }
    }
}

async fn openai_complete(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
    model: &str,
    system: &str,
    user: &str,
    max_tokens: u32,
    cancel: &CancellationToken,
) -> Option<String> {
    let body = json!({
        "model": model,
        "stream": false,
        "max_tokens": max_tokens.max(256),
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user }
        ]
    });
    let url = format!("{}{CHAT_PATH}", provider.base_url);
    let req = client
        .post(&url)
        .bearer_auth(provider.api_key.as_deref().unwrap_or(""))
        .json(&body)
        .timeout(Duration::from_secs(120));
    let resp = tokio::select! {
        r = req.send() => r.ok()?,
        _ = cancel.cancelled() => return None,
    };
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    v.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
}

async fn anthropic_complete(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
    model: &str,
    system: &str,
    user: &str,
    max_tokens: u32,
    cancel: &CancellationToken,
) -> Option<String> {
    let messages: Vec<Message> = vec![Message::system(system), Message::user(user)];
    let mut body =
        message::build_anthropic_request(&messages, &[], "none", &[], max_tokens.max(256));
    body["model"] = json!(model);
    let url = format!("{}{ANTHROPIC_MESSAGES_PATH}", provider.base_url);
    let mut req = client
        .post(&url)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&body)
        .timeout(Duration::from_secs(120));
    if let Some(k) = provider.api_key.as_deref() {
        req = req.header("x-api-key", k);
    }
    for (k, v) in &provider.headers {
        req = req.header(k, v);
    }
    let resp = tokio::select! {
        r = req.send() => r.ok()?,
        _ = cancel.cancelled() => return None,
    };
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    // content is an array of blocks; return the first text block's text.
    v.get("content")
        .and_then(|c| c.as_array())
        .and_then(|blocks| {
            blocks.iter().find_map(|b| {
                (b.get("type").and_then(|t| t.as_str()) == Some("text"))
                    .then(|| b.get("text").and_then(|t| t.as_str()).map(String::from))
                    .flatten()
            })
        })
}

fn fallback_models() -> Vec<ModelInfo> {
    // ponytail: GLM chat template maps any effort except 'high' to 'max', which
    // degenerates thinking output. Advertise only ["high"] so resolve_effort
    // clamps to it — replacing the old hardcoded model-name sniff.
    let std = || {
        DEFAULT_THINKING_LEVELS
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    };
    vec![
        ModelInfo {
            id: "umans-coder".into(),
            name: "Umans Coder".into(),
            reasoning: true,
            context_window: 262144,
            max_tokens: 32768,
            thinking_levels: std(),
            vision: false,

            ..Default::default()
        },
        ModelInfo {
            id: "umans-kimi-k2.5".into(),
            name: "Umans Kimi K2.5".into(),
            reasoning: true,
            context_window: 262144,
            max_tokens: 32768,
            thinking_levels: std(),
            vision: false,

            ..Default::default()
        },
        ModelInfo {
            id: "umans-kimi-k2.6".into(),
            name: "Umans Kimi K2.6".into(),
            reasoning: true,
            context_window: 262144,
            max_tokens: 32768,
            thinking_levels: std(),
            vision: false,

            ..Default::default()
        },
        ModelInfo {
            id: "umans-glm-5.1".into(),
            name: "Umans GLM 5.1".into(),
            reasoning: true,
            context_window: 202752,
            max_tokens: 131072,
            thinking_levels: vec!["high".to_string()],
            vision: false,

            ..Default::default()
        },
        ModelInfo {
            id: "umans-glm-5.2".into(),
            name: "Umans GLM 5.2".into(),
            reasoning: true,
            context_window: 413696,
            max_tokens: 131072,
            thinking_levels: vec!["high".to_string()],
            vision: false,

            ..Default::default()
        },
        ModelInfo {
            id: "umans-minimax-m2.5".into(),
            name: "Umans MiniMax M2.5".into(),
            reasoning: true,
            context_window: 204800,
            max_tokens: 8192,
            thinking_levels: std(),
            vision: false,

            ..Default::default()
        },
    ]
}

/// Discover models live from /models/info; cache to disk with an 8-hour TTL.
/// Falls back to the disk cache (even if stale) on HTTP error, then to the
/// hardcoded snapshot as a last resort.
/// Discover available models for the active provider. Branches on the
/// provider's wire protocol: OpenAI-compatible (`/models/info`) or Anthropic
/// (`/v1/models`). Results are cached to disk, keyed by base URL + kind so an
/// OpenAI and an Anthropic endpoint at the same host don't collide.
pub async fn discover_models(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
) -> Vec<ModelInfo> {
    let cache_key = provider_cache_key(provider);
    match provider.kind {
        ProviderKind::OpenAI => discover_models_openai(client, provider, &cache_key).await,
        ProviderKind::Anthropic => discover_models_anthropic(client, provider, &cache_key).await,
    }
}

/// Cache key: base URL (trailing slash normalized) + provider kind.
fn provider_cache_key(provider: &ResolvedProvider) -> String {
    format!(
        "{}|{}",
        provider.base_url.trim_end_matches('/'),
        provider.kind.as_str()
    )
}

async fn discover_models_openai(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
    cache_key: &str,
) -> Vec<ModelInfo> {
    // OpenCode Go: the single /v1/models endpoint serves every model over both
    // wire protocols with no protocol field, so fetch it live and filter to
    // this provider's protocol (OpenAI chat/completions here). See
    // opencode_go_discover_models for the family-prefix partition + caching.
    if is_opencode_go(&provider.base_url) {
        return opencode_go_discover_models(client, provider, cache_key, true).await;
    }
    // 1. Try disk cache (fresh: < 8 hours old).
    if let Some(models) = read_models_cache(cache_key) {
        return models;
    }

    // 2. Fetch live from the endpoint. Auth is optional here (Umans /models/info
    // is public; custom OpenAI-compatible endpoints may gate it). Send the key
    // only when one is configured so an unauthenticated default still works.
    //
    // `/models/info` is Umans-specific (rich capabilities). First-party and
    // other vanilla OpenAI-compatible endpoints don't serve it, so on a miss
    // we fall back to the standard OpenAI `/models` list and synthesize
    // ModelInfo with curated per-id capabilities.
    let url = format!("{}{MODELS_INFO_PATH}", provider.base_url);
    let mut req = client.get(&url).timeout(Duration::from_secs(5));
    if let Some(k) = provider.api_key.as_deref() {
        req = req.bearer_auth(k);
    }
    let live = match req.send().await {
        Ok(r) if r.status().is_success() => parse_models_response(&match r.json::<Value>().await {
            Ok(v) => v,
            Err(_) => Value::Null,
        }),
        _ => Vec::new(),
    };

    // 2b. /models/info miss (non-Umans endpoint) → standard OpenAI `/models`.
    if live.is_empty() {
        let url = if is_codex_endpoint(&provider.base_url) {
            // The Codex `/models` endpoint REQUIRES `client_version` and filters
            // the catalog by each model's `minimal_client_version`: a value too
            // low (e.g. our own CARGO_PKG_VERSION "0.2.0") returns an EMPTY
            // list, so discovery falls back to a stale hardcoded list and the
            // user ends up sending a slug the backend rejects. The official
            // `codex` CLI sends its own version (>= the latest models' minimum);
            // dev builds send "0.0.0", which the backend special-cases to return
            // the FULL account catalog regardless of minimums. We are not the
            // codex CLI, so use the "0.0.0" dev sentinel — it reliably yields the
            // models this account can actually use (verified: returns 4 models
            // vs 0 for a low non-zero version).
            format!(
                "{}{OPENAI_MODELS_PATH}?client_version=0.0.0",
                provider.base_url
            )
        } else {
            format!("{}{OPENAI_MODELS_PATH}", provider.base_url)
        };
        let mut req = client.get(&url).timeout(Duration::from_secs(8));
        if let Some(k) = provider.api_key.as_deref() {
            req = req.bearer_auth(k);
        }
        for (k, v) in &provider.headers {
            req = req.header(k, v);
        }
        if let Ok(r) = req.send().await {
            if r.status().is_success() {
                if let Ok(v) = r.json::<Value>().await {
                    let listed = if is_codex_endpoint(&provider.base_url) {
                        parse_codex_models_response(&v)
                    } else if is_xai_endpoint(&provider.base_url) {
                        // xAI's `/models` includes context_length, image
                        // pricing, and non-chat media models. Parse with the
                        // xAI-aware path, then enrich vision/chat filter from
                        // `/language-models` when available.
                        let mut models = parse_xai_models_list(&v);
                        if let Some(lang) = fetch_xai_language_model_ids(client, provider).await {
                            apply_xai_language_models_enrichment(&mut models, &lang);
                        }
                        models
                    } else {
                        parse_openai_models_list(&v)
                    };
                    // Enrich with models.dev caps (context/output/reasoning/vision)
                    // for models the curated table left at generic defaults.
                    let mut listed = listed;
                    if let Some(dev) = crate::models_dev::fetch_models_dev(client).await {
                        crate::models_dev::enrich_models(&mut listed, &dev, &provider.base_url);
                    }
                    if !listed.is_empty() {
                        write_models_cache(cache_key, &listed);
                        return listed;
                    }
                }
            }
        }
    }

    if live.is_empty() {
        // Neither endpoint served a usable list — stale cache, else curated
        // fallbacks for the vendor (Gemini host → Gemini models, else Umans).
        return read_models_cache_stale(cache_key)
            .unwrap_or_else(|| openai_fallback_models(&provider.base_url));
    }

    // 3. Write fresh data to disk cache.
    write_models_cache(cache_key, &live);
    live
}

/// Model cache TTL in seconds (8 hours).
const MODELS_CACHE_TTL: u64 = 28800;

/// Cache schema version. Bumped when the parsed model shape OR the cache file
/// shape changes so a stale cache written by an older parser (e.g. one that
/// stored empty thinking_levels or wrong vision flags, or the old single-`key`
/// file shape) is treated as a miss and refreshed, instead of masking the fix
/// for up to the TTL window.
// v7: xAI models parse live `context_length` / vision from `/models` +
// `/language-models` (previously hardcoded wrong windows for Grok).
// v8: Antigravity Gemini 3 + Claude-via-Antigravity catalog.
const MODELS_CACHE_VERSION: u64 = 9;

/// True when a parsed cache object matches the current schema version. Pure
/// (no disk) so the version gate can be unit-tested.
fn cache_version_ok(cache: &Value) -> bool {
    cache.get("version").and_then(|v| v.as_u64()) == Some(MODELS_CACHE_VERSION)
}

fn models_cache_path() -> Option<std::path::PathBuf> {
    let home = crate::config::home_dir()?;
    Some(home.join(".config/catalyst-code/models-cache.json"))
}

fn read_models_cache(cache_key: &str) -> Option<Vec<ModelInfo>> {
    let path = models_cache_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let cache: Value = serde_json::from_str(&content).ok()?;
    if !cache_version_ok(&cache) {
        return None;
    }
    // The cache holds a `key -> entry` map so multiple providers' caches coexist
    // (previously a single `key` field meant each provider's write clobbered the
    // file, so only the last writer ever hit on the next startup).
    let entry = cache.get("entries")?.get(cache_key)?;
    let updated = entry.get("updated_at")?.as_u64()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    if now.saturating_sub(updated) > MODELS_CACHE_TTL {
        return None;
    }
    parse_cache_models(entry)
}

fn read_models_cache_stale(cache_key: &str) -> Option<Vec<ModelInfo>> {
    let path = models_cache_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let cache: Value = serde_json::from_str(&content).ok()?;
    if !cache_version_ok(&cache) {
        return None;
    }
    let entry = cache.get("entries")?.get(cache_key)?;
    parse_cache_models(entry)
}

fn write_models_cache(cache_key: &str, models: &[ModelInfo]) {
    let path = match models_cache_path() {
        Some(p) => p,
        None => return,
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let models_json: Vec<Value> = models
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "name": m.name,
                "reasoning": m.reasoning,
                "context_window": m.context_window,
                "max_tokens": m.max_tokens,
                "thinking_levels": m.thinking_levels,
                "vision": m.vision,
            })
        })
        .collect();
    // Load the existing entries map (if present and same schema) so this
    // provider's entry is MERGED in rather than clobbering the whole file —
    // multi-provider caches then all hit on the next startup instead of only
    // the last writer's. Written atomically (temp + fsync + rename) so a crash
    // mid-write can't truncate/corrupt the cache file.
    // Cross-process lock: the cache is a shared read-modify-write (we merge
    // this provider's entry into the existing entries map). Without a lock two
    // processes refreshing different providers concurrently would both read the
    // same base and the second rename would clobber the first's entry. Advisory
    // (flock); auto-releases on exit/crash so there are no stale locks.
    let _lock = match crate::fsutil::FileLock::acquire(&path.with_extension("lock")) {
        Ok(g) => g,
        Err(_) => return, // best-effort: never block the turn on a wedged lock
    };
    let mut entries: serde_json::Map<String, Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str::<Value>(&c).ok())
        .filter(cache_version_ok)
        .and_then(|c| c.get("entries").cloned())
        .and_then(|e| e.as_object().cloned())
        .unwrap_or_default();
    entries.insert(
        cache_key.to_string(),
        json!({ "updated_at": now, "models": models_json }),
    );
    let cache = json!({
        "version": MODELS_CACHE_VERSION,
        "entries": entries,
    });
    // Unique-temp atomic write (fsutil): two processes never share a temp file,
    // so a concurrent writer can't corrupt this one's write.
    let _ = crate::fsutil::atomic_write_str(
        &path,
        &serde_json::to_string(&cache).unwrap_or_else(|_| "{}".into()),
    );
}

fn parse_cache_models(cache: &Value) -> Option<Vec<ModelInfo>> {
    let arr = cache.get("models")?.as_array()?;
    let mut out = Vec::new();
    for m in arr {
        let id = m.get("id")?.as_str()?.to_string();
        let name = m.get("name")?.as_str()?.to_string();
        let context_window = m.get("context_window")?.as_u64()? as u32;
        let max_tokens = m.get("max_tokens")?.as_u64()? as u32;
        let vision = m.get("vision").and_then(|v| v.as_bool()).unwrap_or(false);
        let thinking_levels = m
            .get("thinking_levels")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        out.push(ModelInfo {
            id,
            name,
            reasoning: m.get("reasoning").and_then(|v| v.as_bool()).unwrap_or(true),
            context_window,
            max_tokens,
            thinking_levels,
            vision,

            ..Default::default()
        });
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Parse the live /models/info response into ModelInfo vec.
fn parse_models_response(data: &Value) -> Vec<ModelInfo> {
    let mut out = Vec::new();
    if let Some(obj) = data.as_object() {
        for (id, info) in obj {
            let caps = info.get("capabilities");
            let cw = caps
                .and_then(|c| c.get("context_window"))
                .and_then(|v| v.as_u64())
                .unwrap_or(200_000) as u32;
            let mt = caps
                .and_then(|c| c.get("recommended_max_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(65000) as u32;
            // Vision comes from capabilities.supports_vision, which the endpoint
            // encodes as true / false / "via-handoff". Only boolean true counts
            // as native client-side vision; "via-handoff" (GLM 5.2, whose vision
            // only works on /v1/messages) falls through to false so the
            // vision-handoff plugin routes image turns to a natively-capable model.
            let vision = caps
                .and_then(|c| c.get("supports_vision"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let name = info
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or(id)
                .to_string();
            // The live /models/info endpoint nests reasoning config under
            // capabilities.reasoning: { supported, can_disable, levels,
            // default_level }. Read levels from there so each model advertises
            // the efforts it actually accepts (e.g. GLM: none/high/max, flash:
            // none/low/medium/high, kimi: []). Flat capability fields
            // (thinking_levels / reasoning_levels / reasoning_efforts) are kept
            // as a fallback for other OpenAI-compatible endpoints.
            let reasoning_caps = caps.and_then(|c| c.get("reasoning"));
            let reasoning_supported = reasoning_caps
                .and_then(|r| r.get("supported"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let thinking_levels = reasoning_caps
                .and_then(|r| r.get("levels"))
                .or_else(|| {
                    caps.and_then(|c| {
                        c.get("thinking_levels")
                            .or_else(|| c.get("reasoning_levels"))
                            .or_else(|| c.get("reasoning_efforts"))
                    })
                })
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            out.push(ModelInfo {
                id: id.clone(),
                name,
                reasoning: reasoning_supported,
                context_window: cw,
                max_tokens: mt,
                thinking_levels,
                vision,

                ..Default::default()
            });
        }
    }
    if out.is_empty() {
        Vec::new()
    } else {
        out
    }
}

/// Parse the standard OpenAI `GET /models` list (`{data:[{id,...}]}`) into
/// ModelInfo, applying curated per-id capabilities for known OpenAI and Gemini
/// model families. Most OpenAI-compatible endpoints return only ids, so we
/// synthesize caps from known families. When the vendor includes richer fields
/// (`context_length`, `context_window`, image token prices), those override the
/// curated defaults — xAI does this on `/v1/models`.
fn parse_openai_models_list(data: &Value) -> Vec<ModelInfo> {
    let Some(arr) = data.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    let mut out: Vec<ModelInfo> = arr
        .iter()
        .filter_map(|m| {
            let id = m.get("id").and_then(|v| v.as_str())?.to_string();
            let name = m
                .get("name")
                .or_else(|| m.get("display_name"))
                .and_then(|v| v.as_str())
                .unwrap_or(&id)
                .to_string();
            let mut info = openai_model_caps(&id, &name);
            apply_live_model_fields(m, &mut info);
            Some(info)
        })
        .collect();
    if out.is_empty() {
        return Vec::new();
    }
    // de-dup by id, preserve order
    let mut seen = std::collections::HashSet::new();
    out.retain(|m| seen.insert(m.id.clone()));
    out
}

/// Overlay vendor-reported fields from a `/models` list item onto curated caps.
/// Safe no-ops when the fields are absent (vanilla OpenAI list).
fn apply_live_model_fields(m: &Value, info: &mut ModelInfo) {
    if let Some(ctx) = m
        .get("context_length")
        .or_else(|| m.get("context_window"))
        .or_else(|| m.get("max_context_window"))
        .or_else(|| m.get("max_model_len"))
        .and_then(|v| v.as_u64())
        .filter(|&c| c > 0)
    {
        info.context_window = ctx.min(u32::MAX as u64) as u32;
        // Keep max_tokens below context so there's room for the prompt.
        if info.max_tokens >= info.context_window {
            info.max_tokens = xai_default_max_tokens(info.context_window);
        }
    }
    if let Some(max) = m
        .get("max_tokens")
        .or_else(|| m.get("max_output_tokens"))
        .or_else(|| m.get("max_completion_tokens"))
        .and_then(|v| v.as_u64())
        .filter(|&c| c > 0)
    {
        info.max_tokens = max.min(u32::MAX as u64) as u32;
    }
    // Image input pricing / modality hints (xAI, some gateways).
    if m.get("prompt_image_token_price")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        > 0
    {
        info.vision = true;
    }
    if let Some(mods) = m.get("input_modalities").and_then(|v| v.as_array()) {
        if mods.iter().any(|x| x.as_str() == Some("image")) {
            info.vision = true;
        }
    }
}

/// Parse xAI `GET /v1/models` into chat ModelInfos, using live `context_length`
/// and filtering out image/video/TTS media models that cannot run the agent loop.
fn parse_xai_models_list(data: &Value) -> Vec<ModelInfo> {
    let Some(arr) = data.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    let mut out: Vec<ModelInfo> = arr
        .iter()
        .filter(|m| is_xai_chat_model_entry(m))
        .filter_map(|m| {
            let id = m.get("id").and_then(|v| v.as_str())?.to_string();
            let name = m
                .get("name")
                .or_else(|| m.get("display_name"))
                .and_then(|v| v.as_str())
                .unwrap_or(&id)
                .to_string();
            let mut info = xai_model_caps(&id, &name);
            apply_live_model_fields(m, &mut info);
            // If context came from the API, re-derive a sensible max_tokens
            // (xAI does not publish max output on this endpoint).
            if m.get("context_length").is_some() {
                info.max_tokens = xai_default_max_tokens(info.context_window);
            }
            Some(info)
        })
        .collect();
    if out.is_empty() {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|m| seen.insert(m.id.clone()));
    sort_xai_models(&mut out);
    out
}

/// True when a `/models` list item is a chat/completions language model (not
/// Grok Imagine image/video or other media-only surfaces).
fn is_xai_chat_model_entry(m: &Value) -> bool {
    let id = m
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if id.is_empty() {
        return false;
    }
    // Media / non-chat surfaces.
    if id.contains("imagine")
        || id.contains("image")
        || id.contains("video")
        || id.contains("tts")
        || id.contains("speech")
        || id.contains("voice")
        || id.contains("embedding")
        || id.contains("whisper")
    {
        return false;
    }
    // Chat models advertise text completion pricing and/or a context window.
    m.get("completion_text_token_price").is_some()
        || m.get("context_length")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            > 0
}

/// Default max output tokens when the vendor does not report one. Keeps a
/// comfortable generation budget while leaving headroom under the context window.
fn xai_default_max_tokens(context_window: u32) -> u32 {
    let headroom = context_window.saturating_sub(4_096).max(8_192);
    headroom.min(65_536)
}

/// Curated xAI Grok capabilities used as the base before live `/models` fields
/// overlay `context_length` / vision. Reasoning is inferred from the model id.
fn xai_model_caps(id: &str, name: &str) -> ModelInfo {
    let l = id.to_ascii_lowercase();
    let std_levels: Vec<String> = DEFAULT_THINKING_LEVELS
        .iter()
        .map(|s| s.to_string())
        .collect();
    // Offline defaults aligned with live xAI catalog (May–Jul 2026). Live
    // `context_length` from `/models` always wins when discovery succeeds.
    let (ctx, reasoning, levels): (u32, bool, Vec<String>) = if l.contains("non-reasoning") {
        (1_000_000, false, Vec::new())
    } else if l.contains("grok-build") {
        (256_000, true, std_levels.clone())
    } else if l.contains("grok-4.5") {
        (500_000, true, std_levels.clone())
    } else if l.contains("grok-4.3") || l.contains("grok-4.20") || l.contains("multi-agent") {
        (1_000_000, true, std_levels.clone())
    } else if l.contains("grok") {
        (256_000, true, std_levels)
    } else {
        (200_000, true, Vec::new())
    };
    // All current Grok chat models accept image inputs (prompt_image_token_price).
    let vision = l.contains("grok");
    ModelInfo {
        id: id.to_string(),
        name: name.to_string(),
        reasoning,
        context_window: ctx,
        max_tokens: xai_default_max_tokens(ctx),
        thinking_levels: levels,
        vision,
        ..Default::default()
    }
}

/// Pin coding default (`grok-build-0.1`) first, then flagship reasoning models.
fn sort_xai_models(models: &mut [ModelInfo]) {
    models.sort_by(|a, b| {
        xai_model_sort_key(&a.id)
            .cmp(&xai_model_sort_key(&b.id))
            .then_with(|| a.id.cmp(&b.id))
    });
}

fn xai_model_sort_key(id: &str) -> u32 {
    let l = id.to_ascii_lowercase();
    if l == "grok-build-0.1" || l.starts_with("grok-build") {
        0
    } else if l.starts_with("grok-4.5") {
        1
    } else if l.starts_with("grok-4.3") {
        2
    } else if l.contains("reasoning") && !l.contains("non-reasoning") {
        3
    } else if l.contains("multi-agent") {
        4
    } else if l.contains("non-reasoning") {
        5
    } else {
        10
    }
}

/// Fetch xAI `GET /v1/language-models` and return a map of model id → whether
/// the model accepts image inputs. Used to drop media-only ids and set vision.
async fn fetch_xai_language_model_ids(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
) -> Option<std::collections::HashMap<String, bool>> {
    let url = format!(
        "{}/language-models",
        provider.base_url.trim_end_matches('/')
    );
    let mut req = client.get(&url).timeout(Duration::from_secs(8));
    if let Some(k) = provider.api_key.as_deref() {
        req = req.bearer_auth(k);
    }
    for (k, v) in &provider.headers {
        req = req.header(k, v);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    let arr = v
        .get("models")
        .or_else(|| v.get("data"))
        .and_then(|d| d.as_array())?;
    let mut map = std::collections::HashMap::new();
    for m in arr {
        let Some(id) = m.get("id").and_then(|x| x.as_str()) else {
            continue;
        };
        let vision = m
            .get("input_modalities")
            .and_then(|mods| mods.as_array())
            .map(|mods| mods.iter().any(|x| x.as_str() == Some("image")))
            .unwrap_or(false)
            || m.get("prompt_image_token_price")
                .and_then(|p| p.as_u64())
                .unwrap_or(0)
                > 0;
        map.insert(id.to_string(), vision);
    }
    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

/// Restrict the discovered list to language-models ids (when that catalog is
/// available) and apply vision flags from `input_modalities`.
fn apply_xai_language_models_enrichment(
    models: &mut Vec<ModelInfo>,
    language: &std::collections::HashMap<String, bool>,
) {
    models.retain(|m| language.contains_key(&m.id));
    for m in models.iter_mut() {
        if let Some(&vision) = language.get(&m.id) {
            m.vision = vision;
        }
    }
    sort_xai_models(models);
}

/// Parse ChatGPT Codex `GET /backend-api/codex/models` (`{models:[...]}`).
/// This is the subscription catalog, so it is the source of truth for which
/// ChatGPT models the logged-in account can actually use.
fn parse_codex_models_response(data: &Value) -> Vec<ModelInfo> {
    let Some(arr) = data.get("models").and_then(|m| m.as_array()) else {
        return Vec::new();
    };
    let mut out: Vec<(Option<u64>, ModelInfo)> = arr
        .iter()
        .filter(|m| {
            // The Codex catalog marks internal/auto models with
            // `visibility: "hide"` (e.g. `codex-auto-review`). These must
            // never be offered or picked as the default — they aren't meant
            // for direct user turns. The official codex CLI excludes them the
            // same way (only `visibility == "list"` models appear in the picker).
            m.get("supported_in_api")
                .and_then(|v| v.as_bool())
                .unwrap_or(true)
                && m.get("visibility").and_then(|v| v.as_str()) != Some("hide")
        })
        .filter_map(|m| {
            let id = m
                .get("slug")
                .or_else(|| m.get("id"))
                .and_then(|v| v.as_str())?;
            let name = m
                .get("display_name")
                .or_else(|| m.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or(id);
            let mut info = openai_model_caps(id, name);
            if let Some(ctx) = m
                .get("context_window")
                .or_else(|| m.get("max_context_window"))
                .and_then(|v| v.as_u64())
            {
                info.context_window = ctx.min(u32::MAX as u64) as u32;
            }
            let levels = m
                .get("supported_reasoning_levels")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| {
                            x.get("effort")
                                .and_then(|v| v.as_str())
                                .or_else(|| x.as_str())
                                .map(String::from)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if !levels.is_empty() {
                info.thinking_levels = levels;
                info.reasoning = true;
            }
            info.vision = m
                .get("supports_image_detail_original")
                .and_then(|v| v.as_bool())
                .unwrap_or(info.vision);
            let priority = m.get("priority").and_then(|v| v.as_u64());
            Some((priority, info))
        })
        .collect();
    // Sort by the catalog's `priority` ascending so the flagship (lowest
    // priority number, e.g. gpt-5.5) lands first and becomes the
    // default-selected model. The official codex CLI picks its default via a
    // separate server `is_default` flag that isn't exposed in this response;
    // priority-ascending is a faithful proxy. Models lacking a priority sort
    // last (stable).
    out.sort_by_key(|(p, _)| p.unwrap_or(u64::MAX));
    let mut out: Vec<ModelInfo> = out.into_iter().map(|(_, m)| m).collect();
    let mut seen = std::collections::HashSet::new();
    out.retain(|m| seen.insert(m.id.clone()));
    out
}

/// Curated capabilities for an OpenAI- or Gemini-family model id. Returns
/// conservative defaults (ctx 200k, max 8k, reasoning true, vision false) for
/// unknown ids so an unrecognized model still works.
#[allow(clippy::if_same_then_else)]
fn openai_model_caps(id: &str, name: &str) -> ModelInfo {
    let l = id.to_ascii_lowercase();
    let std_levels: Vec<String> = DEFAULT_THINKING_LEVELS
        .iter()
        .map(|s| s.to_string())
        .collect();
    // (context_window, max_tokens, reasoning, vision, thinking_levels)
    let (ctx, max, reasoning, vision, levels): (u32, u32, bool, bool, Vec<String>) = if l
        .contains("gpt-5-codex")
    {
        (272_144, 163_840, true, true, std_levels.clone())
    } else if l.contains("gpt-5") {
        (272_144, 128_000, true, true, std_levels.clone())
    } else if l.contains("o4-mini") {
        (200_000, 100_000, true, true, std_levels.clone())
    } else if l.starts_with("o4") || l.contains("o4-") {
        (200_000, 100_000, true, true, std_levels.clone())
    } else if l.starts_with("o3") || l.contains("o3-") {
        (200_000, 100_000, true, false, std_levels.clone())
    } else if l.contains("o1") {
        (200_000, 100_000, true, false, vec!["high".to_string()])
    } else if l.contains("gpt-4.1") {
        (1_047_576, 32_768, false, true, Vec::new())
    } else if l.contains("gpt-4o") {
        (128_000, 16_384, false, true, Vec::new())
    } else if l.contains("gemini-3") && l.contains("flash") {
        // Gemini 3 Flash (Antigravity): thinkingLevel minimal/low/medium/high.
        (
            1_048_576,
            65_536,
            true,
            true,
            vec![
                "minimal".into(),
                "low".into(),
                "medium".into(),
                "high".into(),
            ],
        )
    } else if l.contains("gemini-3") {
        // Gemini 3 / 3.1 Pro (Antigravity): thinkingLevel low/high only.
        (
            1_048_576,
            65_535,
            true,
            true,
            vec!["low".into(), "high".into()],
        )
    } else if l.contains("claude-opus") && l.contains("thinking") {
        // Claude-via-Antigravity (Opus thinking) — 200k ctx.
        (200_000, 64_000, true, true, std_levels.clone())
    } else if l.contains("claude-sonnet-4") || l.contains("claude-opus-4") {
        (200_000, 64_000, false, true, Vec::new())
    } else if l.contains("gemini-2.5-pro") || (l.contains("gemini-2.5") && !l.contains("flash")) {
        (1_048_576, 65_536, true, true, std_levels.clone())
    } else if l.contains("gemini-2.5-flash") {
        (1_048_576, 65_536, true, true, std_levels.clone())
    } else if l.contains("gemini-2.0-flash") {
        (1_048_576, 8_192, false, true, Vec::new())
    } else if l.contains("gemini") {
        (1_048_576, 8_192, false, true, Vec::new())
    } else if l.contains("grok") {
        // Delegate to xai_model_caps so offline OpenAI-list parsing matches
        // the SuperGrok catalog (context_length is overlaid from live API).
        return xai_model_caps(id, name);
    } else {
        (200_000, 8_192, true, false, Vec::new())
    };
    ModelInfo {
        id: id.to_string(),
        name: name.to_string(),
        reasoning,
        context_window: ctx,
        max_tokens: max,
        thinking_levels: levels,
        vision,
        ..Default::default()
    }
}

/// True when the base URL points at Google's Gemini OpenAI-compatible endpoint.
pub fn is_gemini_endpoint(base_url: &str) -> bool {
    let host = base_url
        .split("://")
        .nth(1)
        .unwrap_or(base_url)
        .split(['/', '?'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    host == "generativelanguage.googleapis.com"
}

/// True when the base URL points at a Code Assist / Antigravity gateway
/// (`cloudcode-pa.googleapis.com` or the daily/autopush sandboxes). OAuth-
/// authenticated Gemini/Claude-via-Antigravity requests are routed here —
/// `generativelanguage.googleapis.com` only accepts API keys.
pub fn is_code_assist_endpoint(base_url: &str) -> bool {
    let host = base_url
        .split("://")
        .nth(1)
        .unwrap_or(base_url)
        .split(['/', '?'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    host == "cloudcode-pa.googleapis.com"
        || host == "daily-cloudcode-pa.sandbox.googleapis.com"
        || host == "autopush-cloudcode-pa.sandbox.googleapis.com"
        || (host.ends_with(".sandbox.googleapis.com") && host.contains("cloudcode-pa"))
}

/// True when the base URL points at ChatGPT's Codex subscription backend.
pub fn is_codex_endpoint(base_url: &str) -> bool {
    let host_path = base_url
        .split("://")
        .nth(1)
        .unwrap_or(base_url)
        .trim_end_matches('/')
        .to_ascii_lowercase();
    host_path == "chatgpt.com/backend-api/codex"
        || host_path == "chat.openai.com/backend-api/codex"
        || host_path == "chatgpt-staging.com/backend-api/codex"
}

/// True if the base URL points at an OpenCode Go endpoint. OpenCode Go is a
/// single subscription that serves some models via an OpenAI-compatible
/// `/v1/chat/completions` endpoint and others via an Anthropic `/v1/messages`
/// endpoint — all under one API key at `https://opencode.ai/zen/go/v1`. The
/// harness models this as TWO provider configs (one OpenAI-kind, one
/// Anthropic-kind) sharing the base URL + key; discovery returns a curated,
/// protocol-specific model list for each (see `opencode_go_openai_models` /
/// `opencode_go_anthropic_models`).
pub fn is_opencode_go(base_url: &str) -> bool {
    let host = base_url
        .split("://")
        .nth(1)
        .unwrap_or(base_url)
        .split(['/', '?'])
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    host == "opencode.ai" && base_url.to_ascii_lowercase().contains("/zen/go/")
}

/// True for GitHub Copilot's OpenAI-compatible chat endpoint.
pub fn is_github_copilot_endpoint(base_url: &str) -> bool {
    endpoint_host(base_url) == "api.githubcopilot.com"
}

/// True for Kimi Coding's OpenAI-compatible subscription endpoint.
pub fn is_kimi_coding_endpoint(base_url: &str) -> bool {
    endpoint_host(base_url) == "api.kimi.com" && base_url.to_ascii_lowercase().contains("/coding/")
}

/// True for Kilo Code's OpenRouter-compatible gateway endpoint.
pub fn is_kilocode_endpoint(base_url: &str) -> bool {
    endpoint_host(base_url) == "api.kilo.ai"
}

pub fn is_cline_endpoint(base_url: &str) -> bool {
    endpoint_host(base_url) == "api.cline.bot"
}

/// True when `base_url` points at Anthropic's API (`api.anthropic.com`).
/// The Claude subscription OAuth token must ONLY be sent there — never to a
/// third-party Anthropic-compatible endpoint (a proxy, a local server) — so
/// `enrich_oauth` resolves it only when this is true (not on `kind` alone,
/// which would leak the token to any `kind:"anthropic"` provider).
pub fn is_anthropic_endpoint(base_url: &str) -> bool {
    endpoint_host(base_url) == "api.anthropic.com"
}

pub fn is_kimchi_endpoint(base_url: &str) -> bool {
    let h = endpoint_host(base_url);
    h == "llm.kimchi.dev" || h.ends_with(".kimchi.dev")
}

pub fn is_codebuddy_endpoint(base_url: &str) -> bool {
    endpoint_host(base_url) == "copilot.tencent.com"
}

pub fn is_iflow_endpoint(base_url: &str) -> bool {
    let h = endpoint_host(base_url);
    h == "apis.iflow.cn" || h == "iflow.cn" || h.ends_with(".iflow.cn")
}

fn endpoint_host(base_url: &str) -> String {
    base_url
        .split("://")
        .nth(1)
        .unwrap_or(base_url)
        .split(['/', '?'])
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Capabilities for an OpenCode Go model id. The OpenCode Go `/v1/models`
/// endpoint returns only ids (no context window / max tokens / reasoning /
/// vision), and does NOT indicate which wire protocol each model uses — that
/// mapping lives in the OpenCode docs, not the API — so the harness curates
/// the list. The per-model (context_window, max_tokens, vision) values come
/// from the **Models.dev** registry (`https://models.dev/models.json`) — the
/// same registry OpenCode itself uses — keyed by each model's upstream provider
/// entry (e.g. `zhipuai/glm-5.2`, `minimax/MiniMax-M3`). The OpenCode Go
/// endpoint exposes no richer endpoint of its own (`/v1/models/info` and
/// `/v1/models/{id}` both 404), so Models.dev is the authoritative source.
/// `max_tokens` is the model's max OUTPUT: for the Anthropic-served models
/// (MiniMax/Qwen) the harness sends it as the request `max_tokens` (Anthropic
/// requires the field), so an accurate value avoids truncating long replies;
/// for the OpenAI-served models `max_tokens` is metadata only (the OpenAI path
/// does not send it, so the server applies its own default). `context_window`
/// drives the harness's compaction threshold, so an accurate value keeps
/// compaction from firing far too early on the million-token models.
///
/// Reasoning: for Anthropic-served models (MiniMax/Qwen, per
/// [`opencode_go_model_protocol`]), `thinking_levels` is set to
/// `["low", "medium", "high"]` which enables the standard Anthropic
/// `thinking` block (budgets: 4 096 / 12 288 / 24 576 tokens, clamped below
/// `max_tokens`). The block is only sent when the user picks an effort >
/// "none". For OpenAI-served models (GLM / Kimi / DeepSeek / MiMo),
/// reasoning stays false + levels empty — the OpenAI path only sends
/// `reasoning_effort` for Umans endpoints, and opencode-go is not Umans.
/// For ids not in the table (a model the registry hasn't indexed), fall back
/// to conservative flat defaults.
fn opencode_go_model_caps(id: &str, name: &str) -> ModelInfo {
    let (context_window, max_tokens, vision) =
        opencode_go_caps(id).unwrap_or((200_000, 8_192, false));
    let reasoning;
    let thinking_levels;
    if opencode_go_model_protocol(id) == Some(false) {
        // Anthropic-served models: enable extended thinking via the standard
        // Anthropic `thinking` block. Budgets: low=4096, medium=12288, high=24576
        // (capped below max_tokens by anthropic_thinking_budget).
        reasoning = true;
        thinking_levels = vec!["low".into(), "medium".into(), "high".into()];
    } else {
        // OpenAI-served models: reasoning_effort is only sent for Umans
        // endpoints (opencode-go is not Umans), so no reasoning.
        reasoning = false;
        thinking_levels = Vec::new();
    }
    ModelInfo {
        id: id.to_string(),
        name: name.to_string(),
        reasoning,
        context_window,
        max_tokens,
        thinking_levels,
        vision,
        ..Default::default()
    }
}

/// Real `(context_window, max_tokens, vision)` for each documented OpenCode Go
/// model id, sourced from Models.dev (`https://models.dev/models.json`). Values
/// are the upstream model's limits (OpenCode Go passes the upstream context
/// through, per its tiered pricing for the 256K+/1M models). `vision` is true
/// when the upstream entry's `modalities.input` includes `image`. Returns
/// `None` for ids the registry hasn't indexed; the caller then uses flat
/// defaults. Keep this in sync with [`opencode_go_known_models`] (ids + display
/// names) and [`opencode_go_model_protocol`] (family→wire-protocol routing).
fn opencode_go_caps(id: &str) -> Option<(u32, u32, bool)> {
    let l = id.to_ascii_lowercase();
    Some(match l.as_str() {
        // OpenAI-compatible /v1/chat/completions (zhipu / moonshot / deepseek / xiaomi)
        "glm-5.2" => (1_000_000, 131_072, false),
        "glm-5.1" => (200_000, 131_072, false),
        "kimi-k2.7-code" => (262_144, 262_144, true),
        "kimi-k2.6" => (262_144, 262_144, true),
        "deepseek-v4-pro" => (1_000_000, 384_000, false),
        "deepseek-v4-flash" => (1_000_000, 384_000, false),
        "mimo-v2.5" => (1_048_576, 131_072, true),
        "mimo-v2.5-pro" => (1_048_576, 131_072, false),
        // Anthropic /v1/messages (minimax / alibaba)
        "minimax-m3" => (512_000, 128_000, true),
        "minimax-m2.7" => (204_800, 131_072, false),
        "minimax-m2.5" => (204_800, 131_072, false),
        "qwen3.7-max" => (1_000_000, 65_536, false),
        "qwen3.7-plus" => (1_000_000, 64_000, true),
        "qwen3.6-plus" => (1_000_000, 65_536, true),
        _ => return None,
    })
}

/// All OpenCode Go model ids documented in the OpenCode Go docs endpoint
/// table, paired with their display names. The live `/v1/models` endpoint
/// returns ids without display names or a protocol field, so this table
/// supplies both: the display name (for known ids) and, via the family prefix
/// in [`opencode_go_model_protocol`], the wire protocol. It is also the
/// offline fallback when the endpoint is unreachable.
fn opencode_go_known_models() -> &'static [(&'static str, &'static str)] {
    &[
        // OpenAI-compatible /v1/chat/completions
        ("glm-5.2", "GLM-5.2"),
        ("glm-5.1", "GLM-5.1"),
        ("kimi-k2.7-code", "Kimi K2.7 Code"),
        ("kimi-k2.6", "Kimi K2.6"),
        ("deepseek-v4-pro", "DeepSeek V4 Pro"),
        ("deepseek-v4-flash", "DeepSeek V4 Flash"),
        ("mimo-v2.5", "MiMo-V2.5"),
        ("mimo-v2.5-pro", "MiMo-V2.5-Pro"),
        // Anthropic /v1/messages
        ("minimax-m3", "MiniMax M3"),
        ("minimax-m2.7", "MiniMax M2.7"),
        ("minimax-m2.5", "MiniMax M2.5"),
        ("qwen3.7-max", "Qwen3.7 Max"),
        ("qwen3.7-plus", "Qwen3.7 Plus"),
        ("qwen3.6-plus", "Qwen3.6 Plus"),
    ]
}

/// The wire protocol an OpenCode Go model id is served over, inferred from its
/// family prefix. The `/v1/models` endpoint exposes no protocol field, but the
/// OpenCode Go docs endpoint table partitions cleanly by family:
/// `glm`/`kimi`/`deepseek`/`mimo` → OpenAI (`/v1/chat/completions`);
/// `minimax`/`qwen` → Anthropic (`/v1/messages`). Returns `None` for ids whose
/// family is unknown to the docs (e.g. `hy3-preview`) — those are dropped
/// during discovery rather than misrouted to a protocol they may not speak.
fn opencode_go_model_protocol(id: &str) -> Option<bool> {
    let l = id.to_ascii_lowercase();
    if l.starts_with("glm-")
        || l.starts_with("kimi-")
        || l.starts_with("deepseek-")
        || l.starts_with("mimo-")
    {
        Some(true)
    } else if l.starts_with("minimax-") || l.starts_with("qwen") {
        Some(false)
    } else {
        None
    }
}

/// Display name for an OpenCode Go model id: the curated name from the docs
/// table when known, else synthesized as `Brand <rest>` from the family prefix
/// (so newly-added ids the docs table hasn't caught up to still get a readable
/// name instead of a raw slug).
fn opencode_go_display_name(id: &str) -> String {
    let l = id.to_ascii_lowercase();
    if let Some((_, name)) = opencode_go_known_models().iter().find(|(k, _)| *k == l) {
        return name.to_string();
    }
    let (rest, brand) = if let Some(r) = l.strip_prefix("glm-") {
        (r, "GLM")
    } else if let Some(r) = l.strip_prefix("kimi-") {
        (r, "Kimi")
    } else if let Some(r) = l.strip_prefix("deepseek-") {
        (r, "DeepSeek")
    } else if let Some(r) = l.strip_prefix("mimo-") {
        (r, "MiMo")
    } else if let Some(r) = l.strip_prefix("minimax-") {
        (r, "MiniMax")
    } else if let Some(r) = l.strip_prefix("qwen") {
        (r, "Qwen")
    } else {
        return id.to_string();
    };
    let rest_str: String = rest
        .split('-')
        .map(|tok| {
            let mut c = tok.chars();
            match c.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if rest_str.is_empty() {
        brand.to_string()
    } else {
        format!("{brand} {rest_str}")
    }
}

/// Parse an OpenCode Go `/v1/models` response (`{data:[{id,...}]}`) and keep
/// only the ids served over the given wire protocol, mapping each to curated
/// capabilities. The endpoint lists every model with no protocol field, so we
/// partition by family prefix (see [`opencode_go_model_protocol`]); ids whose
/// family is unknown are dropped (we can't safely route them).
fn opencode_go_filter_models(data: &Value, openai: bool) -> Vec<ModelInfo> {
    let Some(arr) = data.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    let mut out: Vec<ModelInfo> = arr
        .iter()
        .filter_map(|m| {
            let id = m.get("id").and_then(|v| v.as_str())?;
            if opencode_go_model_protocol(id) != Some(openai) {
                return None;
            }
            let name = opencode_go_display_name(id);
            Some(opencode_go_model_caps(id, &name))
        })
        .collect();
    // de-dup by id, preserve order
    let mut seen = std::collections::HashSet::new();
    out.retain(|m| seen.insert(m.id.clone()));
    out
}

/// Discover OpenCode Go models by fetching the single `/v1/models` endpoint
/// (which lists every model over both wire protocols, with no protocol field),
/// filtering to `openai`-protocol models, and caching the result. Falls back to
/// the stale disk cache, then the hardcoded curated list, when the endpoint is
/// unreachable.
///
/// OpenCode Go is modeled as TWO provider configs sharing one base URL + key
/// (OpenAI-kind + Anthropic-kind); this is called for each with `openai`
/// selecting the protocol. The cache key already encodes the kind, so the two
/// partitions never collide.
async fn opencode_go_discover_models(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
    cache_key: &str,
    openai: bool,
) -> Vec<ModelInfo> {
    // 1. Fresh disk cache (< 8h TTL).
    if let Some(models) = read_models_cache(cache_key) {
        return models;
    }
    // 2. Fetch the live OpenAI-style /v1/models list. The endpoint serves every
    //    model here regardless of wire protocol; auth is optional (the list is
    //    public) but we send the key when configured.
    let url = format!("{}{OPENAI_MODELS_PATH}", provider.base_url);
    let mut req = client.get(&url).timeout(Duration::from_secs(8));
    if let Some(k) = provider.api_key.as_deref() {
        req = req.bearer_auth(k);
    }
    for (k, v) in &provider.headers {
        req = req.header(k, v);
    }
    let live = match req.send().await {
        Ok(r) if r.status().is_success() => {
            opencode_go_filter_models(&r.json::<Value>().await.unwrap_or(Value::Null), openai)
        }
        _ => Vec::new(),
    };
    if !live.is_empty() {
        write_models_cache(cache_key, &live);
        return live;
    }
    // 3. Stale cache, else the hardcoded curated list for this protocol.
    read_models_cache_stale(cache_key).unwrap_or_else(|| opencode_go_fallback_models(openai))
}

/// Hardcoded curated list for one protocol — the offline fallback when the
/// OpenCode Go `/v1/models` endpoint is unreachable. Derived from
/// [`opencode_go_known_models`] filtered to the protocol family.
fn opencode_go_fallback_models(openai: bool) -> Vec<ModelInfo> {
    opencode_go_known_models()
        .iter()
        .filter(|(id, _)| opencode_go_model_protocol(id) == Some(openai))
        .map(|(id, name)| opencode_go_model_caps(id, name))
        .collect()
}

/// OpenCode Go models served via the OpenAI-compatible `/v1/chat/completions`
/// endpoint — the offline fallback for the `opencode-go` (OpenAI-kind) provider
/// config. Derived from [`opencode_go_known_models`] filtered to the OpenAI
/// protocol family.
#[allow(dead_code)]
fn opencode_go_openai_models() -> Vec<ModelInfo> {
    opencode_go_fallback_models(true)
}

/// OpenCode Go models served via the Anthropic `/v1/messages` endpoint — the
/// offline fallback for the `opencode-go-anthropic` (Anthropic-kind) provider
/// config. Derived from [`opencode_go_known_models`] filtered to the Anthropic
/// protocol family.
#[allow(dead_code)]
fn opencode_go_anthropic_models() -> Vec<ModelInfo> {
    opencode_go_fallback_models(false)
}

/// Curated fallback models for an OpenAI-compatible endpoint that served no
/// list at all. Gemini host → Gemini models; xAI host → Grok models; otherwise
/// the Umans default list.
fn openai_fallback_models(base_url: &str) -> Vec<ModelInfo> {
    if is_codex_endpoint(base_url) {
        return codex_fallback_models();
    }
    // Code Assist endpoint (OAuth Gemini) and the standard Gemini endpoint both
    // serve the same models — use the Gemini fallback list for both.
    if is_gemini_endpoint(base_url) || is_code_assist_endpoint(base_url) {
        return gemini_fallback_models();
    }
    if is_xai_endpoint(base_url) {
        return xai_fallback_models();
    }
    fallback_models()
}

fn codex_fallback_models() -> Vec<ModelInfo> {
    // Current ChatGPT-subscription Codex model slugs (from the official codex
    // CLI's bundled models.json). These are the source of truth when the live
    // `/backend-api/codex/models` catalog can't be reached. The OLD list
    // (gpt-5.2-codex / gpt-5.1-codex-max / gpt-5-codex) are STALE slugs the
    // backend rejects with "model is not supported when using Codex with a
    // ChatGPT account". Ordered flagship-first so the first entry is the default.
    [
        "gpt-5.5",
        "gpt-5.4",
        "gpt-5.4-mini",
        "gpt-5.3-codex",
        "gpt-5.2",
    ]
    .iter()
    .map(|id| openai_model_caps(id, id))
    .collect()
}

/// Static Antigravity / Gemini model list used when live discovery is
/// unreachable. Antigravity quota models first (Gemini 3 + Claude), then the
/// older Gemini 2.5 models as a secondary set.
fn gemini_fallback_models() -> Vec<ModelInfo> {
    // (id, display_name) — Antigravity model ids match the Code Assist gateway
    // (no "models/" prefix; Gemini 3 Pro uses -low/-high tiers).
    let ids: &[(&str, &str)] = &[
        ("gemini-3.1-pro-high", "Gemini 3.1 Pro (Antigravity)"),
        ("gemini-3-pro-high", "Gemini 3 Pro (Antigravity)"),
        ("gemini-3-flash", "Gemini 3 Flash (Antigravity)"),
        (
            "claude-opus-4-6-thinking",
            "Claude Opus 4.6 Thinking (Antigravity)",
        ),
        ("claude-sonnet-4-6", "Claude Sonnet 4.6 (Antigravity)"),
        ("gemini-2.5-pro", "Gemini 2.5 Pro"),
        ("gemini-2.5-flash", "Gemini 2.5 Flash"),
    ];
    ids.iter()
        .map(|(id, name)| openai_model_caps(id, name))
        .collect()
}

/// Static xAI Grok model list used when `/models` is unreachable. Context
/// windows match the live SuperGrok catalog; `grok-build-0.1` is first.
fn xai_fallback_models() -> Vec<ModelInfo> {
    let ids = [
        "grok-build-0.1",
        "grok-4.5",
        "grok-4.3",
        "grok-4.20-0309-reasoning",
        "grok-4.20-0309-non-reasoning",
        "grok-4.20-multi-agent-0309",
    ];
    let mut models: Vec<ModelInfo> = ids.iter().map(|id| xai_model_caps(id, id)).collect();
    sort_xai_models(&mut models);
    models
}

/// True when the base URL points at xAI's API (`api.x.ai`).
pub fn is_xai_endpoint(base_url: &str) -> bool {
    let host = base_url
        .split("://")
        .nth(1)
        .unwrap_or(base_url)
        .split(['/', '?'])
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    host == "api.x.ai" || host == "x.ai" || host.ends_with(".x.ai")
}

/// True when `base_url` points at Qwen Code's portal chat endpoint
/// (`portal.qwen.ai`). Used by `oauth::enrich_oauth` and presence checks so a
/// user-added provider at that host still picks up the Qwen OAuth token.
pub fn is_qwen_endpoint(base_url: &str) -> bool {
    let host = base_url
        .split("://")
        .nth(1)
        .unwrap_or(base_url)
        .split(['/', '?'])
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    host == "portal.qwen.ai" || host.ends_with(".portal.qwen.ai") || host == "chat.qwen.ai"
}

/// Sanitize orphaned tool_calls: ensure every tool_calls entry has a matching
/// tool result message. Context compaction can drop tool results while keeping
/// the assistant message that made the call, causing a 400. Mirrors the Umans
/// extension's before_provider_request handler.
/// Also verifies that the sanitizer doesn't leave behind a broken conversation
/// (validate that every assistant with tool_calls has corresponding tool results).
#[allow(clippy::ptr_arg)]
pub fn sanitize_orphaned_tool_calls(messages: &mut Vec<Message>) -> usize {
    // Number of fixes applied (orphaned results dropped + synthetic results
    // inserted). Callers persist only when this is non-zero, so clean turns pay
    // just the scan with no session rewrite.
    // All tool_call ids emitted by any assistant message in the kept history.
    let call_ids: std::collections::HashSet<String> = messages
        .iter()
        .filter_map(|m| {
            if m.is_assistant() {
                m.tool_calls()
            } else {
                None
            }
        })
        .flatten()
        .map(|tc| tc.id.clone())
        .collect();

    // All tool_call ids that currently have a matching `role:"tool"` result.
    let result_ids: std::collections::HashSet<String> = messages
        .iter()
        .filter_map(|m| {
            if m.is_tool() {
                m.tool_call_id().map(String::from)
            } else {
                None
            }
        })
        .collect();

    // Drop orphaned RESULTS: a `tool` message whose `tool_call_id` is not
    // emitted by any remaining assistant `tool_calls`. Compaction can keep a
    // tool result while dropping (or summarizing) the assistant call that
    // requested it — OpenAI APIs then reject the orphaned `tool` message with a
    // 400 that bricks the turn (and persists into the next). This is the
    // symmetric fix to the orphaned-CALL handling below.
    let before = messages.len();
    messages.retain(|m| {
        if m.is_tool() {
            m.tool_call_id()
                .map(|id| call_ids.contains(id))
                .unwrap_or(false)
        } else {
            true
        }
    });
    let dropped_results = before - messages.len();

    // Insert synthetic results for orphaned CALLS (assistant tool_calls with no
    // matching tool message). Computed against the original result_ids — the
    // retain above only removed results that had no matching call, so the set
    // of calls-with-results is unchanged.
    let orphaned: Vec<String> = call_ids
        .iter()
        .filter(|id| !result_ids.contains(*id))
        .cloned()
        .collect();
    if orphaned.is_empty() {
        return dropped_results;
    }

    // Insert synthetic tool results right after the assistant message that made each call.
    let mut inserted = 0;
    let mut i = 0;
    while i < messages.len() {
        let is_assistant_with_calls =
            messages[i].is_assistant() && messages[i].tool_calls().is_some();
        if !is_assistant_with_calls {
            i += 1;
            continue;
        }
        let calls: Vec<String> = messages[i]
            .tool_calls()
            .unwrap()
            .iter()
            .map(|tc| tc.id.clone())
            .filter(|id| orphaned.contains(id))
            .collect();
        let insert_at = i + 1;
        for (k, id) in calls.iter().enumerate() {
            messages.insert(
                insert_at + k,
                Message::tool(
                    id,
                    "[tool result was lost — this call did not complete (the turn may have been aborted or its result dropped during context compaction). Re-issue the tool call if still needed.]",
                ),
            );
            inserted += 1;
        }
        i = insert_at + calls.len();
    }
    dropped_results + inserted
}

/// Read a token count from a usage field, tolerating the integer, float, and
/// string encodings different OpenAI-compatible servers emit. `as_u64` alone
/// misses floats (some proxies serialize counts as `100.0`) and quoted numbers,
/// which silently drops the context budget to zero.
/// Sanitize tool-call `arguments`: ensure every assistant tool_call's
/// `arguments` field is a valid JSON string. Some models (notably the GLM
/// family) occasionally emit malformed `arguments` for long, quote-heavy
/// commands wrapped inside `bulk`'s nested JSON. When such a message is
/// replayed in the conversation history, the API rejects the whole request
/// with "Assistant tool call function.arguments must be valid JSON", which
/// then repeats on every subsequent turn and bricks the session. This
/// replaces any malformed `arguments` (and any non-string `arguments`) with
/// the valid string `"{}"` so the history is always API-valid; the matching
/// tool dispatch already returned an actionable error to the model. Returns
/// the number of tool calls fixed.
#[allow(clippy::ptr_arg)]
pub fn sanitize_tool_call_arguments(messages: &mut Vec<Message>) -> usize {
    let mut fixed = 0;
    for m in messages.iter_mut() {
        if !m.is_assistant() {
            continue;
        }
        // Get mutable access to tool_calls via the Message enum
        let calls = match m {
            Message::Assistant {
                tool_calls: Some(ref mut tc),
                ..
            } => tc,
            _ => continue,
        };
        for tc in calls.iter_mut() {
            let malformed = serde_json::from_str::<Value>(&tc.function.arguments).is_err();
            if malformed {
                tc.function.arguments = "{}".to_string();
                fixed += 1;
            }
        }
    }
    fixed
}

fn token_count(v: &Value) -> Option<u64> {
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    if let Some(n) = v.as_f64() {
        return Some(n as u64);
    }
    if let Some(s) = v.as_str() {
        return s.trim().parse::<u64>().ok();
    }
    None
}

/// One streamed assistant turn. Emits `thinking`/`delta`/`tool_call` events as it goes.
/// Retries the initial POST on 429/5xx with exponential backoff (honors Retry-After).
/// Returns the finalized assistant message, finish_reason, and (in/out) token counts.
pub async fn stream_turn(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
    idle_timeout_secs: u64,
    model: &str,
    messages: &[Message],
    tools: &[Value],
    reasoning_effort: &str,
    thinking_levels: &[String],
    max_tokens: u32,
    cancel: &CancellationToken,
    timer: &mut TurnTimer,
    prompt_est: u64,
    quiet: bool,
) -> Result<(Value, String, u64, u64, u64), String> {
    match provider.kind {
        ProviderKind::OpenAI => {
            if is_code_assist_endpoint(&provider.base_url) {
                stream_turn_gemini(
                    client,
                    provider,
                    idle_timeout_secs,
                    model,
                    messages,
                    tools,
                    reasoning_effort,
                    thinking_levels,
                    max_tokens,
                    cancel,
                    timer,
                    prompt_est,
                    quiet,
                )
                .await
            } else if is_codex_endpoint(&provider.base_url) {
                stream_turn_codex(
                    client,
                    provider,
                    idle_timeout_secs,
                    model,
                    messages,
                    tools,
                    reasoning_effort,
                    cancel,
                    timer,
                    prompt_est,
                    quiet,
                )
                .await
            } else {
                stream_turn_openai(
                    client,
                    provider,
                    idle_timeout_secs,
                    model,
                    messages,
                    tools,
                    reasoning_effort,
                    thinking_levels,
                    cancel,
                    timer,
                    prompt_est,
                    quiet,
                )
                .await
            }
        }
        ProviderKind::Anthropic => {
            stream_turn_anthropic(
                client,
                provider,
                idle_timeout_secs,
                model,
                messages,
                tools,
                reasoning_effort,
                thinking_levels,
                max_tokens,
                cancel,
                timer,
                prompt_est,
                quiet,
            )
            .await
        }
    }
}

async fn stream_turn_codex(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
    idle_timeout_secs: u64,
    model: &str,
    messages: &[Message],
    tools: &[Value],
    reasoning_effort: &str,
    cancel: &CancellationToken,
    timer: &mut TurnTimer,
    prompt_est: u64,
    quiet: bool,
) -> Result<(Value, String, u64, u64, u64), String> {
    let api_key = provider.api_key.as_deref().unwrap_or("");
    // Convert Messages → Values for the Codex path (keep existing translator).
    let values = Message::to_openai_messages(messages);
    let (instructions, input) = codex_responses_input(&values);
    let body = json!({
        "model": model,
        "instructions": instructions,
        "input": input,
        "tools": codex_responses_tools(tools),
        "tool_choice": "auto",
        "parallel_tool_calls": true,
        "reasoning": { "effort": reasoning_effort, "summary": "auto" },
        "store": false,
        "stream": true,
        "include": ["reasoning.encrypted_content"],
    });
    let url = format!("{}/responses", provider.base_url.trim_end_matches('/'));
    let resp = send_with_retry(client, &url, api_key, &provider.headers, &body, cancel).await?;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut calls: Vec<ToolAccum> = Vec::new();
    let mut tokens_in = 0;
    let mut tokens_out = 0;
    let mut cached_tokens = 0;
    let idle = Duration::from_secs(idle_timeout_secs.max(10));
    let mut last_stats: Option<Instant> = None;

    loop {
        let chunk = tokio::select! {
            c = tokio::time::timeout(idle, stream.next()) => c.map_err(|_| format!("stream idle timeout ({}s with no data)", idle_timeout_secs))?,
            _ = cancel.cancelled() => return Err("aborted".into()),
        };
        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(|e| format!("stream read: {}", fmt_chain(&e)))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(nl) = buf.find('\n') {
            let line = buf[..nl].trim().to_string();
            buf.drain(..=nl);
            let data = line
                .strip_prefix("data: ")
                .or_else(|| line.strip_prefix("data:"));
            let Some(data) = data else { continue };
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            let Ok(obj) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            match obj.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "response.output_text.delta" => {
                    if let Some(t) = obj.get("delta").and_then(|v| v.as_str()) {
                        if content.is_empty() {
                            timer.mark_first_token();
                        }
                        content.push_str(t);
                        if !quiet {
                            emit(&Event::new("delta").with("text", json!(t)));
                        }
                    }
                }
                "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
                    if let Some(t) = obj.get("delta").and_then(|v| v.as_str()) {
                        if reasoning.is_empty() {
                            timer.mark_first_token();
                        }
                        reasoning.push_str(t);
                        if !quiet {
                            emit(&Event::new("thinking").with("text", json!(t)));
                        }
                    }
                }
                "response.output_item.done" => {
                    if let Some(item) = obj.get("item") {
                        if item.get("type").and_then(|v| v.as_str()) == Some("function_call") {
                            timer.mark_first_token();
                            let call_id = item
                                .get("call_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let name = item
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let args = item
                                .get("arguments")
                                .and_then(|v| v.as_str())
                                .unwrap_or("{}")
                                .to_string();
                            let idx = calls.len();
                            if !quiet {
                                emit(
                                    &Event::new("tool_call_start")
                                        .with("id", json!(call_id))
                                        .with("index", json!(idx)),
                                );
                                emit(
                                    &Event::new("tool_call_name")
                                        .with("index", json!(idx))
                                        .with("name", json!(name)),
                                );
                                emit(
                                    &Event::new("tool_call_args")
                                        .with("index", json!(idx))
                                        .with("args", json!(args)),
                                );
                            }
                            calls.push(ToolAccum {
                                id: call_id,
                                name,
                                args,
                            });
                        }
                    }
                }
                "response.completed" => {
                    if let Some(u) = obj.get("response").and_then(|r| r.get("usage")) {
                        if let Some(p) = u.get("input_tokens").and_then(token_count) {
                            tokens_in = p;
                        }
                        if let Some(o) = u.get("output_tokens").and_then(token_count) {
                            tokens_out = o;
                        }
                        if let Some(c) = u
                            .get("input_tokens_details")
                            .and_then(|d| d.get("cached_tokens"))
                            .and_then(token_count)
                        {
                            cached_tokens = c;
                        }
                    }
                }
                "response.failed" => return Err(format!("Responses API failed: {obj}")),
                _ => {}
            }
            if !quiet && (!content.is_empty() || !reasoning.is_empty()) {
                let now = Instant::now();
                if last_stats
                    .map(|t| now.duration_since(t).as_millis() >= 400)
                    .unwrap_or(true)
                {
                    last_stats = Some(now);
                    let est_out = estimate_tokens(&content) + estimate_tokens(&reasoning);
                    let live_ctx = prompt_est.saturating_add(est_out);
                    let mut ev = Event::new("metrics")
                        .with("tokens_in", json!(live_ctx))
                        .with("tokens_out", json!(est_out))
                        .with("cached_tokens", json!(cached_tokens));
                    if let Some(ttft) = timer
                        .first_token
                        .map(|t| t.duration_since(timer.start).as_millis() as u64)
                    {
                        ev = ev.with("ttft_ms", json!(ttft));
                    }
                    if let Some(tps) = timer.live_tps_estimate(est_out) {
                        ev = ev.with("tps_est", json!(tps));
                    }
                    emit(&ev);
                }
            }
        }
    }
    timer.end_call(
        tokens_out,
        estimate_tokens(&content) + estimate_tokens(&reasoning),
    );
    let tool_calls: Vec<Value> = calls
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "type": "function",
                "function": { "name": c.name, "arguments": c.args }
            })
        })
        .collect();
    let mut msg = serde_json::Map::new();
    msg.insert("role".into(), json!("assistant"));
    if !tool_calls.is_empty() {
        msg.insert("content".into(), Value::Null);
        msg.insert("tool_calls".into(), Value::Array(tool_calls));
    } else {
        msg.insert("content".into(), json!(content));
    }
    Ok((
        Value::Object(msg),
        if calls.is_empty() {
            "stop"
        } else {
            "tool_calls"
        }
        .into(),
        tokens_in,
        tokens_out,
        cached_tokens,
    ))
}

fn codex_responses_input(messages: &[Value]) -> (String, Vec<Value>) {
    let mut instructions = Vec::new();
    let mut input = Vec::new();
    for m in messages {
        match m.get("role").and_then(|v| v.as_str()).unwrap_or("") {
            "system" => instructions.push(content_text(m.get("content").unwrap_or(&Value::Null))),
            "user" => input.push(json!({"type":"message","role":"user","content":[{"type":"input_text","text":content_text(m.get("content").unwrap_or(&Value::Null))}]})),
            "assistant" => {
                if let Some(calls) = m.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in calls {
                        input.push(json!({
                            "type":"function_call",
                            "call_id": tc.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                            "name": tc.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()).unwrap_or(""),
                            "arguments": tc.get("function").and_then(|f| f.get("arguments")).and_then(|v| v.as_str()).unwrap_or("{}"),
                        }));
                    }
                } else {
                    let text = content_text(m.get("content").unwrap_or(&Value::Null));
                    if !text.is_empty() { input.push(json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]})); }
                }
            }
            "tool" => input.push(json!({
                "type":"function_call_output",
                "call_id": m.get("tool_call_id").and_then(|v| v.as_str()).unwrap_or(""),
                "output": content_text(m.get("content").unwrap_or(&Value::Null)),
            })),
            _ => {}
        }
    }
    (instructions.join("\n\n"), input)
}

fn content_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        _ => v.to_string(),
    }
}

fn codex_responses_tools(tools: &[Value]) -> Vec<Value> {
    tools.iter().filter_map(|t| {
        let f = t.get("function")?;
        Some(json!({
            "type": "function",
            "name": f.get("name").cloned().unwrap_or(Value::Null),
            "description": f.get("description").cloned().unwrap_or(Value::Null),
            "parameters": f.get("parameters").cloned().unwrap_or_else(|| json!({"type":"object"})),
            "strict": false,
        }))
    }).collect()
}

/// OpenAI-compatible streaming turn. Emits the same delta/thinking/tool_call
/// events and returns the same (assistant_msg, finish_reason, tokens) tuple
/// as the Anthropic path, so the caller is protocol-agnostic.
async fn stream_turn_openai(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
    idle_timeout_secs: u64,
    model: &str,
    messages: &[Message],
    tools: &[Value],
    reasoning_effort: &str,
    thinking_levels: &[String],
    cancel: &CancellationToken,
    timer: &mut TurnTimer,
    prompt_est: u64,
    quiet: bool,
) -> Result<(Value, String, u64, u64, u64), String> {
    // ponytail: reasoning_effort + reasoning_content replay are Umans-specific.
    // Only emit them when pointed at an Umans endpoint; other OpenAI-compatible
    // servers reject unknown fields with a 400.
    let base_url = &provider.base_url;
    let umans = is_umans(base_url);
    let api_key = provider.api_key.as_deref().unwrap_or("");
    // Convert Messages → OpenAI-shaped JSON for the wire.
    let openai_messages = Message::to_openai_messages(messages);
    let mut body = json!({
        "model": model,
        "messages": openai_messages,
        "tools": tools,
        "tool_choice": "auto",
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if umans {
        // Resolve the requested effort against the model's advertised thinking
        // levels: clamp to the closest supported level when the model constrains
        // the set (e.g. GLM only accepts "high"). Empty levels => pass through.
        let resolved = resolve_effort(reasoning_effort, thinking_levels);
        if resolved != reasoning_effort && !quiet {
            emit(&Event::new("info").with(
                "message",
                json!(format!(
                    "reasoning effort '{}' not supported by model '{}'; using '{}'",
                    reasoning_effort, model, resolved
                )),
            ));
        }
        body["reasoning_effort"] = json!(resolved);
    }

    let url = format!("{base_url}{CHAT_PATH}");

    // ponytail: retry the stream only while NOTHING has been emitted to the TUI
    // yet — once a delta/thinking/tool_call event went out, a retry would
    // duplicate visible output, so we fail instead. The idle + connect timeouts
    // catch stalls; this catches a transient cut *before* the first token.
    let max_attempts = 3u32;
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_calls: Vec<ToolAccum> = Vec::new();
    let mut finish_reason = String::new();
    let mut tokens_in: u64 = 0;
    let mut tokens_out: u64 = 0;
    // ponytail: cached_tokens comes from usage.prompt_tokens_details.cached_tokens
    // (OpenAI/Z.AI implicit prefix caching). Surfaced so the harness can confirm
    // prefix-cache hits and diagnose busts — the request shape is already stable,
    // this just makes the hit visible.
    let mut cached_tokens: u64 = 0;
    // Per-chunk idle timeout: if no bytes arrive for this long mid-stream, abort.
    // Configurable because reasoning models can think >60s before the first token.
    let idle = Duration::from_secs(idle_timeout_secs.max(10));

    // Live stats: the prompt's token count drives the footer's context budget
    // while output streams in (the real `usage` chunk at stream end then
    // overwrites it with exact values). The caller passes the best pre-stream
    // estimate — grounded on the endpoint's last real `prompt_tokens` when one
    // is available, else a char/4 of the whole prompt — so the live percentage
    // tracks reality instead of a whole-conversation char/4 guess.
    let est_prompt = prompt_est;
    let mut last_stats: Option<Instant> = None;

    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let resp = send_with_retry(client, &url, api_key, &provider.headers, &body, cancel).await?;
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        // P2-3: accumulator for a JSON object split across several `data:`
        // lines (some OpenAI-compatible servers do this). A complete object
        // parses on the first line, so the common path is unchanged; only a
        // fragment keeps accumulating until it's whole.
        let mut pending = String::new();
        let mut emitted = false;
        let mut err: Option<String> = None;

        loop {
            let chunk = tokio::select! {
                c = tokio::time::timeout(idle, stream.next()) => match c {
                    Ok(x) => x,
                    Err(_) => { err = Some(format!("stream idle timeout ({}s with no data)", idle_timeout_secs)); break; }
                },
                _ = cancel.cancelled() => return Err("aborted".into()),
            };
            let Some(chunk) = chunk else { break };
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    err = Some(format!("stream read: {}", fmt_chain(&e)));
                    break;
                }
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete SSE frames. A frame may span multiple `data:` lines that
            // must be concatenated before parsing (some OpenAI-compatible servers split).
            while let Some(nl) = buf.find('\n') {
                let line = buf[..nl].trim().to_string();
                buf.drain(..=nl);
                if line.is_empty() {
                    // Blank line = event boundary: drop any half-accumulated frame.
                    pending.clear();
                    continue;
                }
                if line.starts_with(':') {
                    continue; // SSE comment / keepalive
                }
                let data = line
                    .strip_prefix("data: ")
                    .or_else(|| line.strip_prefix("data:"))
                    .unwrap_or("");
                if data == "[DONE]" {
                    pending.clear();
                    continue;
                }
                if data.is_empty() {
                    continue;
                }
                pending.push_str(data);
                let obj = match serde_json::from_str::<Value>(&pending) {
                    Ok(o) => {
                        pending.clear();
                        o
                    }
                    Err(_) => continue, // wait for more `data:` lines to complete the frame
                };

                // usage is sent in a final chunk with an empty choices array.
                // usage is sent in a final chunk with an empty choices array.
                if let Some(u) = obj.get("usage") {
                    if let Some(p) = u.get("prompt_tokens").and_then(token_count) {
                        tokens_in = p;
                    }
                    if let Some(c) = u.get("completion_tokens").and_then(token_count) {
                        tokens_out = c;
                    }
                    // prompt_tokens_details.cached_tokens — the prefix-cache hit count.
                    // Absent on servers that don't support/report caching (stays 0).
                    if let Some(c) = u
                        .get("prompt_tokens_details")
                        .and_then(|d| d.get("cached_tokens"))
                        .and_then(token_count)
                    {
                        cached_tokens = c;
                    }
                }

                let Some(choice) = obj.get("choices").and_then(|c| c.get(0)) else {
                    continue;
                };
                let delta = choice.get("delta");

                if let Some(c) = delta
                    .and_then(|d| d.get("content"))
                    .and_then(|v| v.as_str())
                {
                    if !c.is_empty() {
                        if content.is_empty() {
                            timer.mark_first_token();
                        }
                        content.push_str(c);
                        if !quiet {
                            emitted = true;
                            emit(&Event::new("delta").with("text", json!(c)));
                        }
                    }
                }
                if let Some(r) = delta
                    .and_then(|d| d.get("reasoning_content"))
                    .and_then(|v| v.as_str())
                {
                    if !r.is_empty() {
                        if reasoning.is_empty() {
                            timer.mark_first_token();
                        }
                        reasoning.push_str(r);
                        if !quiet {
                            emitted = true;
                            emit(&Event::new("thinking").with("text", json!(r)));
                        }
                    }
                }
                if let Some(tcs) = delta
                    .and_then(|d| d.get("tool_calls"))
                    .and_then(|v| v.as_array())
                {
                    if !tcs.is_empty() {
                        timer.mark_first_token();
                    }
                    for tc in tcs {
                        let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        while tool_calls.len() <= idx {
                            tool_calls.push(ToolAccum::default());
                        }
                        let acc = &mut tool_calls[idx];
                        if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                            if acc.id.is_empty() {
                                acc.id = id.to_string();
                                if !quiet {
                                    emitted = true;
                                    emit(
                                        &Event::new("tool_call_start")
                                            .with("id", json!(id))
                                            .with("index", json!(idx)),
                                    );
                                }
                            }
                        }
                        let func = tc.get("function");
                        if let Some(name) =
                            func.and_then(|f| f.get("name")).and_then(|v| v.as_str())
                        {
                            if acc.name.is_empty() {
                                acc.name = name.to_string();
                                if !quiet {
                                    emitted = true;
                                    emit(
                                        &Event::new("tool_call_name")
                                            .with("index", json!(idx))
                                            .with("name", json!(name)),
                                    );
                                }
                            }
                        }
                        if let Some(args) = func
                            .and_then(|f| f.get("arguments"))
                            .and_then(|v| v.as_str())
                        {
                            acc.args.push_str(args);
                            if !quiet {
                                emitted = true;
                                emit(
                                    &Event::new("tool_call_args")
                                        .with("index", json!(idx))
                                        .with("args", json!(args)),
                                );
                            }
                        }
                    }
                }
                if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                    if !fr.is_empty() {
                        finish_reason = fr.to_string();
                    }
                }
            }

            // Live footer stats: emit a metrics event at most every ~400ms so the
            // TUI's context + approximate in-flight TPS move during the turn.
            // `tps_est` is explicitly marked approximate by the TUI; the final
            // `tps` still uses provider-reported usage only.
            if !quiet && (!content.is_empty() || !reasoning.is_empty()) {
                let now = Instant::now();
                let due = last_stats
                    .map(|t| now.duration_since(t) >= Duration::from_millis(400))
                    .unwrap_or(true);
                if due {
                    last_stats = Some(now);
                    let est_out = estimate_tokens(&content) + estimate_tokens(&reasoning);
                    let live_ctx = est_prompt.saturating_add(est_out);
                    let mut ev = Event::new("metrics")
                        .with("tokens_in", json!(live_ctx))
                        .with("tokens_out", json!(est_out));
                    if let Some(ttft) = timer
                        .first_token
                        .map(|t| t.duration_since(timer.start).as_millis() as u64)
                    {
                        ev = ev.with("ttft_ms", json!(ttft));
                    }
                    if let Some(tps) = timer.live_tps_estimate(est_out) {
                        ev = ev.with("tps_est", json!(tps));
                    }
                    emit(&ev);
                }
            }
        }

        if err.is_none() {
            break; // stream completed cleanly
        }
        let msg = err.unwrap();
        // Retry only if we showed nothing to the TUI yet (else output duplicates).
        if emitted || attempt >= max_attempts {
            return Err(msg);
        }
        let backoff = backoff_ms(attempt, None);
        emit(
            &Event::new("http_retry")
                .with("attempt", json!(attempt))
                .with("reason", json!("stream error before first token"))
                .with("backoff_ms", json!(backoff)),
        );
        // Reset accumulators for the fresh attempt.
        content.clear();
        reasoning.clear();
        tool_calls.clear();
        finish_reason.clear();
        tokens_in = 0;
        tokens_out = 0;
        cached_tokens = 0;
        timer.call_first_token = None;
        sleep_or_cancel(Duration::from_millis(backoff), cancel).await?;
    }

    // Fold this call's generation time + output tokens into the turn totals so
    // finalize() computes TPS over generation time only (excluding tool-call
    // wait and prefill). est_out is the char/4 fallback numerator when the
    // endpoint omits usage.
    let est_out = estimate_tokens(&content) + estimate_tokens(&reasoning);
    timer.end_call(tokens_out, est_out);

    // Build the assistant message. OpenAI requires content null when tool_calls
    // present and empty. reasoning_content is Umans-only (gated above).
    let mut msg = serde_json::Map::new();
    msg.insert("role".into(), json!("assistant"));
    msg.insert(
        "content".into(),
        if content.is_empty() {
            Value::Null
        } else {
            json!(content)
        },
    );
    if umans && !reasoning.is_empty() {
        msg.insert("reasoning_content".into(), json!(reasoning));
    }
    if !tool_calls.is_empty() {
        let arr: Vec<Value> = tool_calls
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "arguments": if t.args.is_empty() { "{}".to_string() } else { t.args.clone() }
                    }
                })
            })
            .collect();
        msg.insert("tool_calls".into(), json!(arr));
    }

    Ok((
        Value::Object(msg),
        finish_reason,
        tokens_in,
        tokens_out,
        cached_tokens,
    ))
}

// ===========================================================================
// Antigravity / Code Assist API (daily-cloudcode-pa / cloudcode-pa)
// ===========================================================================
//
// When a user signs in via the Antigravity OAuth flow, the OAuth token is for
// the Code Assist / Antigravity gateway — NOT for generativelanguage.googleapis.com
// (which only accepts API keys). The gateway uses the native Google GenAI wire
// format (not OpenAI-compatible), so we need our own message converter, request
// builder, and SSE response parser. Gemini 3 + Claude-via-Antigravity ride the
// same path.

/// Map a user-facing / catalog model id onto the Antigravity Code Assist wire
/// id. Strips `models/` and `antigravity-` prefixes; for Gemini 3 Pro (which
/// requires a `-low`/`-high` tier suffix on Antigravity) appends the tier from
/// the requested reasoning effort when missing.
fn resolve_antigravity_model_id(model: &str, reasoning_effort: &str) -> String {
    let mut id = model.strip_prefix("models/").unwrap_or(model).to_string();
    if let Some(rest) = id.strip_prefix("antigravity-") {
        id = rest.to_string();
    }
    let lower = id.to_ascii_lowercase();
    // Gemini 3 / 3.1 Pro on Antigravity requires an explicit -low/-high tier.
    let is_pro = (lower.starts_with("gemini-3") || lower.starts_with("gemini-3.1"))
        && lower.contains("pro")
        && !lower.contains("flash");
    let has_tier = lower.ends_with("-low") || lower.ends_with("-high");
    if is_pro && !has_tier {
        let tier = match reasoning_effort.to_ascii_lowercase().as_str() {
            "low" | "minimal" | "none" | "" => "low",
            _ => "high",
        };
        id = format!("{id}-{tier}");
    }
    id
}

/// Apply the right thinkingConfig shape for the Antigravity model family.
fn apply_antigravity_thinking(request: &mut Value, model: &str, reasoning_effort: &str) {
    let lower = model.to_ascii_lowercase();
    let effort = reasoning_effort.to_ascii_lowercase();
    let off = matches!(effort.as_str(), "" | "none" | "off");
    if lower.contains("gemini-3") {
        // Gemini 3 uses thinkingLevel strings, not numeric budgets.
        if off {
            // Still send a low level — Gemini 3 rejects budget 0; "low" is the
            // cheapest tier Antigravity accepts for Pro, "minimal" for Flash.
            let level = if lower.contains("flash") {
                "minimal"
            } else {
                "low"
            };
            request["request"]["generationConfig"]["thinkingConfig"] =
                json!({ "thinkingLevel": level, "includeThoughts": true });
        } else {
            let level = match effort.as_str() {
                "minimal" => "minimal",
                "low" => "low",
                "medium" => "medium",
                "high" | "max" => "high",
                _ => {
                    if lower.contains("flash") {
                        "medium"
                    } else {
                        "high"
                    }
                }
            };
            // Pro only accepts low/high — clamp medium/minimal.
            let level = if !lower.contains("flash") {
                match level {
                    "high" => "high",
                    _ => "low",
                }
            } else {
                level
            };
            request["request"]["generationConfig"]["thinkingConfig"] =
                json!({ "thinkingLevel": level, "includeThoughts": true });
        }
        return;
    }
    // Gemini 2.5 / Claude-via-Antigravity: numeric budget.
    if off {
        request["request"]["generationConfig"]["thinkingConfig"] = json!({ "thinkingBudget": 0 });
    } else {
        let budget = match effort.as_str() {
            "low" | "minimal" => 8192,
            "medium" => 16384,
            "high" | "max" => 32768,
            _ => 16384,
        };
        request["request"]["generationConfig"]["thinkingConfig"] =
            json!({ "thinkingBudget": budget, "includeThoughts": true });
    }
}

/// Convert `&[Message]` to the Code Assist (native GenAI) `contents` array.
/// Returns (contents, systemInstruction). System messages are extracted into a
/// separate `systemInstruction` field (the GenAI API doesn't put them in
/// `contents`). Tool-result messages need the function NAME (not just
/// tool_call_id), so we track the last assistant's tool_call id→name map.
fn messages_to_genai_contents(messages: &[Message]) -> (Vec<Value>, Option<Value>) {
    let mut contents = Vec::new();
    let mut system_parts: Vec<Value> = Vec::new();
    let mut last_tool_call_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for msg in messages {
        match msg {
            Message::System { content, .. } => {
                let text = genai_content_to_text(content);
                if !text.is_empty() {
                    system_parts.push(json!({"text": text}));
                }
            }
            Message::User { content, .. } => {
                let text = genai_content_to_text(content);
                contents.push(json!({"role": "user", "parts": [{"text": text}]}));
            }
            Message::Assistant {
                content,
                tool_calls,
                ..
            } => {
                last_tool_call_names.clear();
                let mut parts: Vec<Value> = Vec::new();
                if let Some(text) = content {
                    if !text.is_empty() {
                        parts.push(json!({"text": text}));
                    }
                }
                if let Some(tcs) = tool_calls {
                    for tc in tcs {
                        last_tool_call_names.insert(tc.id.clone(), tc.function.name.clone());
                        let args: Value =
                            serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
                        parts.push(
                            json!({"functionCall": {"name": &tc.function.name, "args": args}}),
                        );
                    }
                }
                if !parts.is_empty() {
                    contents.push(json!({"role": "model", "parts": parts}));
                }
            }
            Message::Tool {
                tool_call_id,
                name,
                content,
            } => {
                let func_name = name
                    .clone()
                    .or_else(|| last_tool_call_names.get(tool_call_id).cloned())
                    .unwrap_or_else(|| "unknown".to_string());
                contents.push(json!({
                    "role": "function",
                    "parts": [{"functionResponse": {"name": func_name, "response": {"result": content}}}]
                }));
            }
        }
    }

    let system_instruction = if system_parts.is_empty() {
        None
    } else {
        Some(json!({"parts": system_parts}))
    };
    (contents, system_instruction)
}

/// Extract plain text from a `Content` (string or multimodal — joins text parts).
fn genai_content_to_text(content: &crate::message::Content) -> String {
    use crate::message::{Content, ContentPart};
    match content {
        Content::Text(s) => s.clone(),
        Content::Multimodal(parts) => parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text } => Some(text.clone()),
                ContentPart::Image { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Convert OpenAI-shaped tool schemas to GenAI `tools` format.
/// OpenAI: `[{"type":"function","function":{"name":..,"description":..,"parameters":..}}]`
/// GenAI:  `[{"functionDeclarations":[{"name":..,"description":..,"parameters":..}]}]`
fn tools_to_genai(tools: &[Value]) -> Vec<Value> {
    let decls: Vec<Value> = tools
        .iter()
        .filter_map(|t| t.get("function"))
        .map(|f| {
            let mut d = json!({
                "name": f.get("name").cloned().unwrap_or(json!("")),
                "description": f.get("description").cloned().unwrap_or(json!("")),
            });
            if let Some(p) = f.get("parameters") {
                d["parameters"] = p.clone();
            }
            d
        })
        .collect();
    if decls.is_empty() {
        Vec::new()
    } else {
        vec![json!({"functionDeclarations": decls})]
    }
}

/// Stream a turn through the Antigravity / Code Assist API (native GenAI
/// wire format). This is the OAuth path for Gemini 3 + Claude-via-Antigravity
/// — `generativelanguage.googleapis.com` only accepts API keys; the OAuth
/// token authenticates against the daily/prod `cloudcode-pa` gateways.
async fn stream_turn_gemini(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
    idle_timeout_secs: u64,
    model: &str,
    messages: &[Message],
    tools: &[Value],
    reasoning_effort: &str,
    _thinking_levels: &[String],
    max_tokens: u32,
    cancel: &CancellationToken,
    timer: &mut TurnTimer,
    prompt_est: u64,
    quiet: bool,
) -> Result<(Value, String, u64, u64, u64), String> {
    let api_key = provider.api_key.as_deref().unwrap_or("");
    let base_url = provider.base_url.trim_end_matches('/');
    let max_attempts = 3u32;

    // Onboarding: get the Code Assist project ID (cached for process lifetime).
    let project = crate::oauth::code_assist_project(client)
        .await
        .ok_or_else(|| {
            "Code Assist onboarding failed: could not obtain a project ID. \
             Try /login again or check your Google account."
                .to_string()
        })?;

    // Convert messages + tools to GenAI format.
    let (contents, system_instruction) = messages_to_genai_contents(messages);
    let genai_tools = tools_to_genai(tools);

    // Strip "models/" / "antigravity-" prefixes; resolve Gemini 3 Pro tier.
    let model_name = resolve_antigravity_model_id(model, reasoning_effort);

    // Build the request body. `userAgent: "antigravity"` is required by the
    // Antigravity gateway (CLIProxy / opencode-antigravity-auth send it).
    let mut request = json!({
        "model": model_name,
        "project": project,
        "userAgent": "antigravity",
        "request": {
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": max_tokens,
            },
        },
    });
    if let Some(si) = system_instruction {
        request["request"]["systemInstruction"] = si;
    }
    if !genai_tools.is_empty() {
        request["request"]["tools"] = json!(genai_tools);
    }
    // Thinking config:
    // - Gemini 3: thinkingLevel string (minimal/low/medium/high)
    // - Gemini 2.5 / Claude-via-Antigravity: thinkingBudget numeric + includeThoughts
    // - "none"/empty: disable (budget 0) for non-Gemini-3 families
    apply_antigravity_thinking(&mut request, &model_name, reasoning_effort);

    let url = format!("{base_url}:streamGenerateContent?alt=sse");
    let idle = Duration::from_secs(idle_timeout_secs.max(5));
    let est_prompt = prompt_est;
    let mut last_stats: Option<Instant> = None;

    let mut content = String::new();
    let mut reasoning = String::new();
    let mut genai_tool_calls: Vec<(String, Value)> = Vec::new(); // (name, args)
    let mut finish_reason = String::new();
    let mut tokens_in: u64 = 0;
    let mut tokens_out: u64 = 0;
    let mut cached_tokens: u64 = 0;
    let mut emitted = false;

    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let resp =
            send_with_retry(client, &url, api_key, &provider.headers, &request, cancel).await?;
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut pending = String::new();
        let mut err: Option<String> = None;

        loop {
            let chunk = tokio::select! {
                c = tokio::time::timeout(idle, stream.next()) => match c {
                    Ok(x) => x,
                    Err(_) => { err = Some(format!("stream idle timeout ({}s with no data)", idle_timeout_secs)); break; }
                },
                _ = cancel.cancelled() => return Err("aborted".into()),
            };
            let Some(chunk) = chunk else { break };
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    err = Some(format!("stream read: {}", fmt_chain(&e)));
                    break;
                }
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(nl) = buf.find('\n') {
                let line = buf[..nl].trim().to_string();
                buf.drain(..=nl);
                if line.is_empty() || line.starts_with(':') {
                    pending.clear();
                    continue;
                }
                let data = line
                    .strip_prefix("data: ")
                    .or_else(|| line.strip_prefix("data:"))
                    .unwrap_or("");
                if data == "[DONE]" || data.is_empty() {
                    pending.clear();
                    continue;
                }
                pending.push_str(data);
                let obj = match serde_json::from_str::<Value>(&pending) {
                    Ok(o) => {
                        pending.clear();
                        o
                    }
                    Err(_) => continue,
                };

                // Code Assist wraps the GenAI response in a "response" field.
                let resp_obj = obj.get("response").unwrap_or(&obj);

                // Usage metadata (may arrive on any chunk, finalized on the last).
                if let Some(u) = resp_obj.get("usageMetadata") {
                    if let Some(p) = u.get("promptTokenCount").and_then(token_count) {
                        tokens_in = p;
                    }
                    if let Some(c) = u.get("candidatesTokenCount").and_then(token_count) {
                        tokens_out = c;
                    }
                    if let Some(t) = u.get("cachedContentTokenCount").and_then(token_count) {
                        cached_tokens = t;
                    }
                }

                let Some(candidate) = resp_obj.get("candidates").and_then(|c| c.get(0)) else {
                    continue;
                };

                // Parse content parts (text / thought / functionCall).
                if let Some(parts) = candidate
                    .get("content")
                    .and_then(|c| c.get("parts"))
                    .and_then(|p| p.as_array())
                {
                    for part in parts {
                        // Regular text content.
                        if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                            let is_thought = part
                                .get("thought")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            if !t.is_empty() {
                                if is_thought {
                                    if reasoning.is_empty() {
                                        timer.mark_first_token();
                                    }
                                    reasoning.push_str(t);
                                    if !quiet {
                                        emitted = true;
                                        emit(&Event::new("thinking").with("text", json!(t)));
                                    }
                                } else {
                                    if content.is_empty() {
                                        timer.mark_first_token();
                                    }
                                    content.push_str(t);
                                    if !quiet {
                                        emitted = true;
                                        emit(&Event::new("delta").with("text", json!(t)));
                                    }
                                }
                            }
                        }
                        // Function call (tool call).
                        if let Some(fc) = part.get("functionCall") {
                            let name = fc
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let args = fc.get("args").cloned().unwrap_or(json!({}));
                            genai_tool_calls.push((name.clone(), args.clone()));
                            if !quiet {
                                emitted = true;
                                emit(
                                    &Event::new("tool_call_name")
                                        .with("index", json!(genai_tool_calls.len() - 1))
                                        .with("name", json!(name)),
                                );
                                emit(
                                    &Event::new("tool_call_args")
                                        .with("index", json!(genai_tool_calls.len() - 1))
                                        .with("args", json!(args.to_string())),
                                );
                            }
                        }
                    }
                }

                if let Some(fr) = candidate.get("finishReason").and_then(|v| v.as_str()) {
                    if !fr.is_empty() && fr != "FINISH_REASON_UNSPECIFIED" {
                        finish_reason = fr.to_string();
                    }
                }

                // Live footer stats.
                if !quiet && (!content.is_empty() || !reasoning.is_empty()) {
                    let now = Instant::now();
                    let due = last_stats
                        .map(|t| now.duration_since(t) >= Duration::from_millis(400))
                        .unwrap_or(true);
                    if due {
                        last_stats = Some(now);
                        let est_out = estimate_tokens(&content) + estimate_tokens(&reasoning);
                        let live_ctx = est_prompt.saturating_add(est_out);
                        let mut ev = Event::new("metrics")
                            .with("tokens_in", json!(live_ctx))
                            .with("tokens_out", json!(est_out));
                        if let Some(ttft) = timer
                            .first_token
                            .map(|t| t.duration_since(timer.start).as_millis() as u64)
                        {
                            ev = ev.with("ttft_ms", json!(ttft));
                        }
                        if let Some(tps) = timer.live_tps_estimate(est_out) {
                            ev = ev.with("tps_est", json!(tps));
                        }
                        emit(&ev);
                    }
                }
            }
        }

        if err.is_none() {
            break;
        }
        let msg = err.unwrap();
        if emitted || attempt >= max_attempts {
            return Err(msg);
        }
        let backoff = backoff_ms(attempt, None);
        emit(
            &Event::new("http_retry")
                .with("attempt", json!(attempt))
                .with("reason", json!("stream error before first token"))
                .with("backoff_ms", json!(backoff)),
        );
        content.clear();
        reasoning.clear();
        genai_tool_calls.clear();
        finish_reason.clear();
        tokens_in = 0;
        tokens_out = 0;
        cached_tokens = 0;
        timer.call_first_token = None;
        sleep_or_cancel(Duration::from_millis(backoff), cancel).await?;
    }

    let est_out = estimate_tokens(&content) + estimate_tokens(&reasoning);
    timer.end_call(tokens_out, est_out);

    // Build the assistant message in OpenAI shape (the rest of the harness
    // expects OpenAI-format messages).
    let mut msg = serde_json::Map::new();
    msg.insert("role".into(), json!("assistant"));
    msg.insert(
        "content".into(),
        if content.is_empty() {
            Value::Null
        } else {
            json!(content)
        },
    );
    if !reasoning.is_empty() {
        msg.insert("reasoning_content".into(), json!(reasoning));
    }
    if !genai_tool_calls.is_empty() {
        let arr: Vec<Value> = genai_tool_calls
            .iter()
            .enumerate()
            .map(|(i, (name, args))| {
                json!({
                    "id": format!("call_{i}"),
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": args.to_string(),
                    }
                })
            })
            .collect();
        msg.insert("tool_calls".into(), json!(arr));
    }

    // Map GenAI finish reasons to OpenAI finish reasons.
    let finish = match finish_reason.as_str() {
        "STOP" => "stop",
        "MAX_TOKENS" => "length",
        "SAFETY" | "RECITATION" => "content_filter",
        _ => "stop",
    };

    Ok((
        Value::Object(msg),
        finish.to_string(),
        tokens_in,
        tokens_out,
        cached_tokens,
    ))
}

/// HMAC-SHA256 (RFC 2104) over `payload` with `key`, hex-encoded.
/// Implemented with sha2 so we don't need an extra `hmac` crate.
fn hmac_sha256_hex(key: &[u8], payload: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    const BLOCK: usize = 64;
    let mut k = if key.len() > BLOCK {
        Sha256::digest(key).to_vec()
    } else {
        key.to_vec()
    };
    k.resize(BLOCK, 0);
    let mut ipad = [0u8; BLOCK];
    let mut opad = [0u8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] = k[i] ^ 0x36;
        opad[i] = k[i] ^ 0x5c;
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(payload);
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner.finalize());
    outer
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// iFlow chat requests require per-request HMAC headers (session-id,
/// x-iflow-timestamp, x-iflow-signature) matching 9router's IFlowExecutor.
fn iflow_signed_headers(api_key: &str, headers: &[(String, String)]) -> Vec<(String, String)> {
    use rand::RngCore;
    let mut uuid = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut uuid);
    // RFC 4122 variant bits for a random UUID (v4-ish).
    uuid[6] = (uuid[6] & 0x0f) | 0x40;
    uuid[8] = (uuid[8] & 0x3f) | 0x80;
    let session_id = format!(
        "session-{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        uuid[0], uuid[1], uuid[2], uuid[3], uuid[4], uuid[5], uuid[6], uuid[7],
        uuid[8], uuid[9], uuid[10], uuid[11], uuid[12], uuid[13], uuid[14], uuid[15]
    );
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let user_agent = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
        .map(|(_, v)| v.as_str())
        .unwrap_or("iFlow-Cli");
    let payload = format!("{user_agent}:{session_id}:{timestamp}");
    let signature = hmac_sha256_hex(api_key.as_bytes(), payload.as_bytes());
    let mut out = headers.to_vec();
    // Drop any stale signature headers so retries re-sign cleanly.
    out.retain(|(k, _)| {
        let kl = k.to_ascii_lowercase();
        kl != "session-id" && kl != "x-iflow-timestamp" && kl != "x-iflow-signature"
    });
    out.push(("session-id".into(), session_id));
    out.push(("x-iflow-timestamp".into(), timestamp.to_string()));
    out.push(("x-iflow-signature".into(), signature));
    out
}

/// POST with retry on 429/5xx. Exponential backoff: 0.5s, 1s, 2s, 4s (cap 8s),
/// honoring Retry-After if present. Up to 4 attempts. Cancellation-aware.
async fn send_with_retry(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    headers: &[(String, String)],
    body: &Value,
    cancel: &CancellationToken,
) -> Result<reqwest::Response, String> {
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        // ponytail: no total .timeout() here. It's a *total* timeout covering
        // connect+headers+the entire body read, so a reasoning turn (GLM @ high)
        // that streams >5 min gets aborted mid-stream with "operation timed out".
        // Stalls are caught by connect_timeout (connect phase, on the client) +
        // the per-chunk idle timeout in stream_turn (body phase).
        // iFlow requires a fresh HMAC signature on every request (9router
        // IFlowExecutor). Re-sign on each attempt so retries stay valid.
        let signed: Vec<(String, String)>;
        let headers = if is_iflow_endpoint(url) {
            signed = iflow_signed_headers(api_key, headers);
            signed.as_slice()
        } else {
            headers
        };
        let mut req = client.post(url).bearer_auth(api_key).json(body);
        for (k, v) in headers {
            req = req.header(k, v);
        }

        let resp = tokio::select! {
            r = req.send() => r,
            _ = cancel.cancelled() => return Err("aborted".into()),
        };

        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                // Transport error: retry with backoff.
                if attempt >= 4 {
                    return Err(format!(
                        "request failed after {attempt} attempts: {}",
                        fmt_chain(&e)
                    ));
                }
                let backoff = backoff_ms(attempt, None);
                emit(
                    &Event::new("http_retry")
                        .with("attempt", json!(attempt))
                        .with("reason", json!("transport error"))
                        .with("backoff_ms", json!(backoff)),
                );
                sleep_or_cancel(Duration::from_millis(backoff), cancel).await?;
                continue;
            }
        };

        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }

        // Retryable: 429 (rate limit) and 5xx (server). 4xx otherwise → fatal.
        let retryable = status.as_u16() == 429 || status.is_server_error();
        if !retryable || attempt >= 4 {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("HTTP {status}: {text}"));
        }

        // P2-6: Retry-After may be integer seconds OR an HTTP-date; parse both.
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_retry_after);
        // Drain body before retry to free the connection.
        let _ = resp.text().await;
        let backoff = backoff_ms(attempt, retry_after);
        emit(
            &Event::new("http_retry")
                .with("attempt", json!(attempt))
                .with("status", json!(status.as_u16()))
                .with("backoff_ms", json!(backoff)),
        );
        sleep_or_cancel(Duration::from_millis(backoff), cancel).await?;
    }
}

/// Parse a Retry-After header into seconds. Accepts an integer (seconds) or
/// an HTTP-date (RFC 7231 IMF-fixdate, e.g. "Wed, 21 Oct 2025 07:28:00 GMT");
/// the latter is converted to seconds-from-now (clamped >= 0). Returns None for
/// anything unparseable so the caller falls back to exponential backoff.
fn parse_retry_after(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Ok(n) = s.parse::<u64>() {
        return Some(n);
    }
    let date = parse_http_date(s)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let diff = date.saturating_sub(now);
    if diff == 0 {
        None
    } else {
        Some(diff)
    }
}

/// Parse an HTTP IMF-fixdate ("Wed, 21 Oct 2025 07:28:00 GMT") into UNIX
/// seconds. The weekday is ignored (servers sometimes send the wrong one).
fn parse_http_date(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 5 {
        return None;
    }
    let day: u32 = parts[1].trim_end_matches(',').parse().ok()?;
    let mon: u32 = match parts[2] {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i32 = parts[3].parse().ok()?;
    let tparts: Vec<&str> = parts[4].split(':').collect();
    if tparts.len() != 3 {
        return None;
    }
    let h: u64 = tparts[0].parse().ok()?;
    let mi: u64 = tparts[1].parse().ok()?;
    let se: u64 = tparts[2].parse().ok()?;
    let days = days_from_civil(year, mon, day)?;
    Some(days * 86400 + h * 3600 + mi * 60 + se)
}

/// Days since the UNIX epoch (1970-01-01) for a proleptic Gregorian date.
/// Howard Hinnant's days_from_civil algorithm; valid for any year.
fn days_from_civil(y: i32, m: u32, d: u32) -> Option<u64> {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let m_shift = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * (m_shift as i64) + 2) / 5 + (d as i64) - 1;
    let doe = (yoe as i64) * 365 + (yoe as i64) / 4 - (yoe as i64) / 100 + doy;
    let days = (era as i64) * 146097 + doe - 719468;
    if days < 0 {
        return None;
    }
    Some(days as u64)
}

fn backoff_ms(attempt: u32, retry_after: Option<u64>) -> u64 {
    if let Some(ra) = retry_after {
        return ra.saturating_mul(1000).min(30_000);
    }
    // 500, 1000, 2000, 4000 ... capped at 8000
    let base = 500u64;
    base.saturating_mul(1u64 << (attempt - 1)).min(8000)
}

async fn sleep_or_cancel(d: Duration, cancel: &CancellationToken) -> Result<(), String> {
    tokio::select! {
        _ = tokio::time::sleep(d) => Ok(()),
        _ = cancel.cancelled() => Err("aborted".into()),
    }
}

#[derive(Default)]
struct ToolAccum {
    id: String,
    name: String,
    args: String,
}

fn fmt_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(c) = src {
        s.push_str(" -> ");
        s.push_str(&c.to_string());
        src = c.source();
    }
    s
}

// =========================================================================
// Anthropic Messages API translation
// =========================================================================
//
// The harness keeps the conversation in OpenAI chat-completions shape. These
// functions translate OpenAI messages + tools -> an Anthropic `/v1/messages`
// request, and an Anthropic SSE stream -> the same delta/thinking/tool_call
// events the OpenAI path emits, then rebuild the assistant message in OpenAI
// shape. The rest of the harness never sees Anthropic wire format.

/// Map a reasoning effort to an Anthropic extended-thinking token budget.
/// Returns None when thinking can't be enabled (effort "none"/unknown, or
/// `max_tokens` too small to leave room for a >=1024 budget — Anthropic counts
/// thinking within `max_tokens`, so the budget must be < max_tokens).
#[allow(dead_code)]
fn anthropic_thinking_budget(effort: &str, max_tokens: u32) -> Option<u32> {
    let base: u32 = match effort.to_ascii_lowercase().as_str() {
        "low" | "minimal" => 4096,
        "medium" => 12288,
        "high" | "max" => 24576,
        _ => return None,
    };
    let budget = base.min(max_tokens.saturating_sub(1024));
    if budget < 1024 {
        return None;
    }
    Some(budget)
}

/// Push text from an OpenAI `content` (string or multimodal array) into a vec
/// of system-parts. Image parts are ignored (system is text-only).
#[allow(dead_code)]
fn push_content_str(content: &Value, parts: &mut Vec<String>) {
    if let Some(s) = content.as_str() {
        if !s.is_empty() {
            parts.push(s.to_string());
        }
    } else if let Some(arr) = content.as_array() {
        for part in arr {
            if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                    if !t.is_empty() {
                        parts.push(t.to_string());
                    }
                }
            }
        }
    }
}

/// Append a message with the given role + content blocks, merging into the
/// previous message when it has the same role (Anthropic requires alternating
/// roles; consecutive same-role messages 400). Merging concatenates the block
/// arrays — e.g. several OpenAI `role:tool` results fold into one user message
/// with multiple `tool_result` blocks.
#[allow(dead_code)]
fn push_or_merge(out: &mut Vec<Value>, role: &str, blocks: Vec<Value>) {
    if let Some(last) = out.last_mut() {
        if last.get("role").and_then(|r| r.as_str()) == Some(role) {
            if let Some(arr) = last.get_mut("content").and_then(|c| c.as_array_mut()) {
                arr.extend(blocks);
                return;
            }
        }
    }
    out.push(json!({ "role": role, "content": blocks }));
}

/// Convert a single OpenAI message `content` (string or multimodal array) into
/// Anthropic content blocks. Images become Anthropic `image` blocks (base64 or
/// url source); text stays text. A plain string yields a single text block.
#[allow(dead_code)]
fn anthropic_content_blocks(content: &Value) -> Vec<Value> {
    if let Some(s) = content.as_str() {
        return vec![json!({ "type": "text", "text": s })];
    }
    let mut blocks = Vec::new();
    if let Some(arr) = content.as_array() {
        for part in arr {
            match part.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                        blocks.push(json!({ "type": "text", "text": t }));
                    }
                }
                Some("image_url") => {
                    if let Some(url) = part
                        .get("image_url")
                        .and_then(|iu| iu.get("url"))
                        .and_then(|u| u.as_str())
                    {
                        if let Some(img) = anthropic_image_block(url) {
                            blocks.push(img);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if blocks.is_empty() {
        blocks.push(json!({ "type": "text", "text": "" }));
    }
    blocks
}

/// Build an Anthropic `image` block from an OpenAI `image_url.url`. Supports
/// `data:<media>;base64,<data>` (-> base64 source) and plain URLs (-> url source).
#[allow(dead_code)]
fn anthropic_image_block(url: &str) -> Option<Value> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',')?;
        let media = meta.split(';').next()?;
        Some(json!({
            "type": "image",
            "source": { "type": "base64", "media_type": media, "data": data }
        }))
    } else {
        Some(json!({ "type": "image", "source": { "type": "url", "url": url } }))
    }
}

/// Convert OpenAI function tools to Anthropic tool definitions.
/// OpenAI: `{"type":"function","function":{"name","description","parameters"}}`
/// Anthropic: `{"name","description","input_schema"}`
#[allow(dead_code)]
fn anthropic_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|t| {
            let f = t.get("function")?;
            let name = f.get("name").and_then(|v| v.as_str())?;
            let description = f.get("description").and_then(|v| v.as_str()).unwrap_or("");
            let schema = f.get("parameters").cloned().unwrap_or_else(|| json!({}));
            Some(json!({ "name": name, "description": description, "input_schema": schema }))
        })
        .collect()
}

/// Build an Anthropic `/v1/messages` request body from OpenAI-shaped messages +
/// tools. Extracts `role: system` messages into the top-level `system` field,
/// converts user/assistant/tool messages to Anthropic format, and converts
/// OpenAI function tools to `input_schema` tools. `thinking_levels` non-empty +
/// a supported effort enables extended thinking. Pure (no I/O) so it can be
/// unit-tested directly.
///
/// **DEPRECATED**: Use `message::build_anthropic_request(messages: &[Message], ...)`
/// instead — it works on typed `Message` values rather than opaque `Value` JSON.
/// This function is kept for backward-compat with existing tests and will be
/// removed once callers are migrated.
#[allow(dead_code)]
pub fn build_anthropic_request(
    messages: &[Value],
    tools: &[Value],
    model: &str,
    reasoning_effort: &str,
    thinking_levels: &[String],
    max_tokens: u32,
) -> Value {
    let mut system_parts: Vec<String> = Vec::new();
    let mut out: Vec<Value> = Vec::new();
    for m in messages {
        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("");
        match role {
            "system" => {
                push_content_str(m.get("content").unwrap_or(&Value::Null), &mut system_parts)
            }
            "user" => {
                let blocks = anthropic_content_blocks(m.get("content").unwrap_or(&Value::Null));
                push_or_merge(&mut out, "user", blocks);
            }
            "assistant" => {
                let mut blocks = Vec::new();
                if let Some(content) = m.get("content") {
                    if let Some(s) = content.as_str() {
                        if !s.is_empty() {
                            blocks.push(json!({ "type": "text", "text": s }));
                        }
                    } else if let Some(arr) = content.as_array() {
                        for part in arr {
                            if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                                if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                                    if !t.is_empty() {
                                        blocks.push(json!({ "type": "text", "text": t }));
                                    }
                                }
                            }
                        }
                    }
                }
                // assistant tool_calls -> tool_use blocks. reasoning_content is
                // dropped: Anthropic can't replay raw thinking without matching
                // signatures (it would 400), so prior reasoning is never sent back.
                if let Some(tcs) = m.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in tcs {
                        let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let func = tc.get("function").cloned().unwrap_or_else(|| json!({}));
                        let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let args = func
                            .get("arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}");
                        let input: Value = serde_json::from_str(args).unwrap_or_else(|_| json!({}));
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": input
                        }));
                    }
                }
                if blocks.is_empty() {
                    blocks.push(json!({ "type": "text", "text": "" }));
                }
                push_or_merge(&mut out, "assistant", blocks);
            }
            "tool" => {
                // OpenAI tool result -> Anthropic user message with a tool_result block.
                let tool_use_id = m.get("tool_call_id").and_then(|v| v.as_str()).unwrap_or("");
                let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
                push_or_merge(
                    &mut out,
                    "user",
                    vec![json!({
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": content
                    })],
                );
            }
            _ => {}
        }
    }

    let mut body = serde_json::Map::new();
    body.insert("model".into(), json!(model));
    body.insert("max_tokens".into(), json!(max_tokens));
    if !system_parts.is_empty() {
        body.insert("system".into(), json!(system_parts.join("\n\n")));
    }
    if !out.is_empty() {
        body.insert("messages".into(), Value::Array(out));
    }
    if !tools.is_empty() {
        body.insert("tools".into(), Value::Array(anthropic_tools(tools)));
        body.insert("tool_choice".into(), json!({ "type": "auto" }));
    }
    if !thinking_levels.is_empty() {
        // Only enable extended thinking when the user actually asked for it.
        // `resolve_effort` would otherwise clamp "none" up to a supported level
        // and silently turn thinking on; gate on the raw requested effort first.
        let wants = !matches!(
            reasoning_effort.to_ascii_lowercase().as_str(),
            "" | "none" | "minimal" | "off"
        );
        if wants {
            let resolved = resolve_effort(reasoning_effort, thinking_levels);
            if let Some(budget) = anthropic_thinking_budget(&resolved, max_tokens) {
                body.insert(
                    "thinking".into(),
                    json!({ "type": "enabled", "budget_tokens": budget }),
                );
            }
        }
    }
    Value::Object(body)
}

/// Map an Anthropic `stop_reason` to the OpenAI `finish_reason` the harness
/// expects ("stop" | "tool_calls" | "length").
fn anthropic_stop_reason(sr: &str) -> String {
    match sr {
        "end_turn" | "stop_sequence" => "stop".to_string(),
        "tool_use" => "tool_calls".to_string(),
        "max_tokens" => "length".to_string(),
        other => other.to_string(),
    }
}

/// Accumulator for one Anthropic content block while streaming (text / thinking
/// / tool_use). Keyed by the block `index` from the SSE events.
#[derive(Default)]
struct AnthropicBlock {
    kind: String,
    tool_id: String,
    tool_name: String,
    tool_args: String,
}

/// Initialize a `tool_use` block from a `content_block_start` event's
/// `content_block` object — sets the tool id + name ONLY. The streamed
/// arguments arrive separately via `input_json_delta` fragments (handled in
/// the `content_block_delta` arm), so the start event's `input` field — which
/// Anthropic's streaming API always sends as the empty placeholder `{}` for a
/// tool_use — must NOT be captured here. Prepending it would corrupt the
/// assembled arguments as `{}{...}` (e.g. `{}{"command":"ls"}`), which the
/// tool dispatcher then rejects as malformed JSON. This was observed with
/// MiniMax-M3 over the OpenCode Go Anthropic `/v1/messages` path. Empty args
/// are substituted with `"{}"` downstream when finalizing tool_calls, so
/// leaving `tool_args` empty here is correct.
fn init_tool_use_block(b: &mut AnthropicBlock, cb: &Value) {
    b.tool_id = cb
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    b.tool_name = cb
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
}

/// POST an Anthropic request with retry on 429/5xx (same policy as the OpenAI
/// path). Auth is `x-api-key` (not Bearer); `anthropic-version` + any provider
/// headers are attached. Cancellation-aware.
async fn send_anthropic_request(
    client: &reqwest::Client,
    url: &str,
    provider: &ResolvedProvider,
    body: &Value,
    cancel: &CancellationToken,
) -> Result<reqwest::Response, String> {
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let mut req = client
            .post(url)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(body);
        if provider.oauth {
            // Claude.ai subscription (OAuth): Bearer token + the claude-code
            // identity beta (Anthropic's gateway requires it for subscription
            // tokens). UA/x-app come from provider.headers (injected by
            // enrich_oauth). Reuses the same Messages endpoint as the API-key path.
            if let Some(k) = provider.api_key.as_deref() {
                req = req.header("authorization", format!("Bearer {k}"));
            }
            req = req.header("anthropic-beta", crate::oauth::CLAUDE_OAUTH_BETA);
        } else if let Some(k) = provider.api_key.as_deref() {
            req = req.header("x-api-key", k);
        }
        for (k, v) in &provider.headers {
            req = req.header(k, v);
        }
        let resp = tokio::select! {
            r = req.send() => r,
            _ = cancel.cancelled() => return Err("aborted".into()),
        };
        match resp {
            Ok(r) => {
                let status = r.status();
                if status.is_success() {
                    return Ok(r);
                }
                let retryable = status.as_u16() == 429 || status.is_server_error();
                if !retryable || attempt >= 4 {
                    let text = r.text().await.unwrap_or_default();
                    return Err(format!("HTTP {status}: {text}"));
                }
                let retry_after = r
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(parse_retry_after);
                let _ = r.text().await;
                let backoff = backoff_ms(attempt, retry_after);
                emit(
                    &Event::new("http_retry")
                        .with("attempt", json!(attempt))
                        .with("status", json!(status.as_u16()))
                        .with("backoff_ms", json!(backoff)),
                );
                sleep_or_cancel(Duration::from_millis(backoff), cancel).await?;
            }
            Err(e) => {
                if attempt >= 4 {
                    return Err(format!(
                        "request failed after {attempt} attempts: {}",
                        fmt_chain(&e)
                    ));
                }
                let backoff = backoff_ms(attempt, None);
                emit(
                    &Event::new("http_retry")
                        .with("attempt", json!(attempt))
                        .with("reason", json!("transport error"))
                        .with("backoff_ms", json!(backoff)),
                );
                sleep_or_cancel(Duration::from_millis(backoff), cancel).await?;
            }
        }
    }
}

/// Anthropic streaming turn. Emits the same delta/thinking/tool_call events
/// and returns the same (assistant_msg, finish_reason, tokens) tuple as
/// `stream_turn_openai`, so the caller is protocol-agnostic.
async fn stream_turn_anthropic(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
    idle_timeout_secs: u64,
    model: &str,
    messages: &[Message],
    tools: &[Value],
    reasoning_effort: &str,
    thinking_levels: &[String],
    max_tokens: u32,
    cancel: &CancellationToken,
    timer: &mut TurnTimer,
    prompt_est: u64,
    quiet: bool,
) -> Result<(Value, String, u64, u64, u64), String> {
    let mt = if max_tokens == 0 { 8192 } else { max_tokens };
    // Use the native Message-based Anthropic request builder.
    let mut body =
        message::build_anthropic_request(messages, tools, reasoning_effort, thinking_levels, mt);
    body["stream"] = json!(true);
    body["model"] = json!(model);

    let url = format!("{}{ANTHROPIC_MESSAGES_PATH}", provider.base_url);
    let idle = Duration::from_secs(idle_timeout_secs.max(10));
    // Live stats: same grounded prompt estimate as the OpenAI path; the real
    // `usage` at stream end overwrites the footer with exact values.
    let est_prompt = prompt_est;
    let mut last_stats: Option<Instant> = None;

    let mut content = String::new();
    let mut reasoning = String::new();
    let mut blocks: Vec<AnthropicBlock> = Vec::new();
    let mut finish_reason = String::new();
    let mut tokens_in: u64 = 0;
    let mut tokens_out: u64 = 0;
    let mut cached_tokens: u64 = 0;

    let max_attempts = 3u32;
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let resp = send_anthropic_request(client, &url, provider, &body, cancel).await?;
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut cur_event = String::new();
        let mut pending = String::new();
        let mut emitted = false;
        let mut err: Option<String> = None;

        loop {
            let chunk = tokio::select! {
                c = tokio::time::timeout(idle, stream.next()) => match c {
                    Ok(x) => x,
                    Err(_) => {
                        err = Some(format!("stream idle timeout ({}s with no data)", idle_timeout_secs));
                        break;
                    }
                },
                _ = cancel.cancelled() => return Err("aborted".into()),
            };
            let Some(chunk) = chunk else { break };
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    err = Some(format!("stream read: {}", fmt_chain(&e)));
                    break;
                }
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete SSE frames. Anthropic frames pair an `event:`
            // line with a `data:` line; the event type drives dispatch.
            while let Some(nl) = buf.find('\n') {
                let line = buf[..nl].trim().to_string();
                buf.drain(..=nl);
                if line.is_empty() {
                    pending.clear();
                    cur_event.clear();
                    continue;
                }
                if line.starts_with(':') {
                    continue; // SSE comment / keepalive
                }
                if let Some(ev) = line.strip_prefix("event:") {
                    cur_event = ev.trim().to_string();
                    continue;
                }
                let data = line
                    .strip_prefix("data: ")
                    .or_else(|| line.strip_prefix("data:"))
                    .unwrap_or("");
                if data.is_empty() {
                    continue;
                }
                if data == "[DONE]" {
                    pending.clear();
                    continue;
                }
                pending.push_str(data);
                let obj = match serde_json::from_str::<Value>(&pending) {
                    Ok(o) => {
                        pending.clear();
                        o
                    }
                    Err(_) => continue, // wait for more `data:` lines to complete the frame
                };

                match cur_event.as_str() {
                    "message_start" => {
                        if let Some(u) = obj.get("message").and_then(|m| m.get("usage")) {
                            if let Some(p) = u.get("input_tokens").and_then(token_count) {
                                tokens_in = p;
                            }
                            if let Some(c) = u.get("cache_read_input_tokens").and_then(token_count)
                            {
                                cached_tokens = c;
                            }
                        }
                    }
                    "content_block_start" => {
                        let idx = obj.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        while blocks.len() <= idx {
                            blocks.push(AnthropicBlock::default());
                        }
                        let cb = obj.get("content_block").cloned().unwrap_or(Value::Null);
                        let btype = cb
                            .get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or("text")
                            .to_string();
                        let b = &mut blocks[idx];
                        b.kind = btype.clone();
                        if btype == "tool_use" {
                            // tool id + name only; the streamed `input` arrives
                            // via input_json_delta (see init_tool_use_block).
                            timer.mark_first_token();
                            init_tool_use_block(b, &cb);
                            if !quiet {
                                emitted = true;
                                emit(
                                    &Event::new("tool_call_start")
                                        .with("id", json!(b.tool_id))
                                        .with("index", json!(idx)),
                                );
                                if !b.tool_name.is_empty() {
                                    emit(
                                        &Event::new("tool_call_name")
                                            .with("index", json!(idx))
                                            .with("name", json!(b.tool_name)),
                                    );
                                }
                            }
                        }
                    }
                    "content_block_delta" => {
                        let idx = obj.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        while blocks.len() <= idx {
                            blocks.push(AnthropicBlock::default());
                        }
                        let Some(delta) = obj.get("delta") else {
                            continue;
                        };
                        let dtype = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match dtype {
                            "text_delta" => {
                                if let Some(t) = delta.get("text").and_then(|v| v.as_str()) {
                                    if !t.is_empty() {
                                        if content.is_empty() {
                                            timer.mark_first_token();
                                        }
                                        content.push_str(t);
                                        blocks[idx].kind = "text".into();
                                        if !quiet {
                                            emitted = true;
                                            emit(&Event::new("delta").with("text", json!(t)));
                                        }
                                    }
                                }
                            }
                            "thinking_delta" => {
                                if let Some(t) = delta.get("thinking").and_then(|v| v.as_str()) {
                                    if !t.is_empty() {
                                        if reasoning.is_empty() {
                                            timer.mark_first_token();
                                        }
                                        reasoning.push_str(t);
                                        blocks[idx].kind = "thinking".into();
                                        if !quiet {
                                            emitted = true;
                                            emit(&Event::new("thinking").with("text", json!(t)));
                                        }
                                    }
                                }
                            }
                            "input_json_delta" => {
                                if let Some(pj) = delta.get("partial_json").and_then(|v| v.as_str())
                                {
                                    if !pj.is_empty() {
                                        timer.mark_first_token();
                                    }
                                    let b = &mut blocks[idx];
                                    if b.kind.is_empty() {
                                        b.kind = "tool_use".into();
                                    }
                                    b.tool_args.push_str(pj);
                                    if !quiet {
                                        emitted = true;
                                        emit(
                                            &Event::new("tool_call_args")
                                                .with("index", json!(idx))
                                                .with("args", json!(pj)),
                                        );
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    "content_block_stop" => { /* block complete; nothing to emit */ }
                    "message_delta" => {
                        if let Some(d) = obj.get("delta") {
                            if let Some(sr) = d.get("stop_reason").and_then(|v| v.as_str()) {
                                finish_reason = anthropic_stop_reason(sr);
                            }
                        }
                        if let Some(u) = obj.get("usage") {
                            if let Some(o) = u.get("output_tokens").and_then(token_count) {
                                tokens_out = o;
                            }
                        }
                    }
                    "message_stop" | "ping" => { /* keepalive / done */ }
                    "error" => {
                        let msg = obj
                            .get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(|m| m.as_str())
                            .unwrap_or("anthropic stream error")
                            .to_string();
                        err = Some(msg);
                        break;
                    }
                    _ => {}
                }

                // Live footer stats (same ~400ms throttle as the OpenAI path).
                if !quiet && (!content.is_empty() || !reasoning.is_empty()) {
                    let now = Instant::now();
                    let due = last_stats
                        .map(|t| now.duration_since(t) >= Duration::from_millis(400))
                        .unwrap_or(true);
                    if due {
                        last_stats = Some(now);
                        let est_out = estimate_tokens(&content) + estimate_tokens(&reasoning);
                        let live_ctx = est_prompt.saturating_add(est_out);
                        let mut ev = Event::new("metrics")
                            .with("tokens_in", json!(live_ctx))
                            .with("tokens_out", json!(est_out));
                        if let Some(ttft) = timer
                            .first_token
                            .map(|t| t.duration_since(timer.start).as_millis() as u64)
                        {
                            ev = ev.with("ttft_ms", json!(ttft));
                        }
                        if let Some(tps) = timer.live_tps_estimate(est_out) {
                            ev = ev.with("tps_est", json!(tps));
                        }
                        emit(&ev);
                    }
                }
            }
        }

        if err.is_none() {
            break; // stream completed cleanly
        }
        let msg = err.unwrap();
        if emitted || attempt >= max_attempts {
            return Err(msg);
        }
        let backoff = backoff_ms(attempt, None);
        emit(
            &Event::new("http_retry")
                .with("attempt", json!(attempt))
                .with("reason", json!("stream error before first token"))
                .with("backoff_ms", json!(backoff)),
        );
        content.clear();
        reasoning.clear();
        blocks.clear();
        finish_reason.clear();
        tokens_in = 0;
        tokens_out = 0;
        cached_tokens = 0;
        timer.call_first_token = None;
        sleep_or_cancel(Duration::from_millis(backoff), cancel).await?;
    }

    let est_out = estimate_tokens(&content) + estimate_tokens(&reasoning);
    timer.end_call(tokens_out, est_out);

    // Rebuild the assistant message in OpenAI shape. reasoning is shown live but
    // NOT persisted: Anthropic thinking blocks aren't replayable (would 400 next
    // turn), so we drop them from history — same as the OpenAI path drops
    // reasoning_content on non-Umans endpoints.
    let mut msg = serde_json::Map::new();
    msg.insert("role".into(), json!("assistant"));
    msg.insert(
        "content".into(),
        if content.is_empty() {
            Value::Null
        } else {
            json!(content)
        },
    );
    let tool_calls: Vec<Value> = blocks
        .iter()
        .filter(|b| b.kind == "tool_use")
        .map(|b| {
            json!({
                "id": b.tool_id,
                "type": "function",
                "function": {
                    "name": b.tool_name,
                    "arguments": if b.tool_args.is_empty() {
                        "{}".to_string()
                    } else {
                        b.tool_args.clone()
                    }
                }
            })
        })
        .collect();
    if !tool_calls.is_empty() {
        msg.insert("tool_calls".into(), json!(tool_calls));
    }

    Ok((
        Value::Object(msg),
        finish_reason,
        tokens_in,
        tokens_out,
        cached_tokens,
    ))
}

/// Discover models from an Anthropic-compatible endpoint (`GET /v1/models`).
/// Anthropic lists model ids but not capabilities, so each id is mapped through
/// a curated capability table; unknown ids get conservative defaults.
async fn discover_models_anthropic(
    client: &reqwest::Client,
    provider: &ResolvedProvider,
    cache_key: &str,
) -> Vec<ModelInfo> {
    // OpenCode Go: the single /v1/models endpoint serves every model over both
    // wire protocols with no protocol field, so fetch it live and filter to
    // this provider's protocol (Anthropic /v1/messages here). See
    // opencode_go_discover_models for the family-prefix partition + caching.
    if is_opencode_go(&provider.base_url) {
        return opencode_go_discover_models(client, provider, cache_key, false).await;
    }
    if let Some(models) = read_models_cache(cache_key) {
        return models;
    }
    let url = format!("{}{ANTHROPIC_MODELS_PATH}", provider.base_url);
    let mut req = client.get(&url).timeout(Duration::from_secs(8));
    if provider.oauth {
        if let Some(k) = provider.api_key.as_deref() {
            req = req.header("authorization", format!("Bearer {k}"));
        }
        req = req.header("anthropic-beta", crate::oauth::CLAUDE_OAUTH_BETA);
    } else if let Some(k) = provider.api_key.as_deref() {
        req = req.header("x-api-key", k);
    }
    req = req.header("anthropic-version", ANTHROPIC_VERSION);
    for (k, v) in &provider.headers {
        req = req.header(k, v);
    }
    let mut live = match req.send().await {
        Ok(r) if r.status().is_success() => {
            parse_anthropic_models(&r.json::<Value>().await.unwrap_or_else(|_| json!({})))
        }
        _ => read_models_cache_stale(cache_key).unwrap_or_else(anthropic_fallback_models),
    };
    // Enrich with models.dev caps for models the curated table left at
    // generic defaults (relevant for Anthropic-compatible gateways).
    if let Some(dev) = crate::models_dev::fetch_models_dev(client).await {
        crate::models_dev::enrich_models(&mut live, &dev, &provider.base_url);
    }
    write_models_cache(cache_key, &live);
    live
}

/// Parse Anthropic `GET /v1/models` -> `{data:[{id,display_name,...}]}` into
/// ModelInfo, applying curated per-id capabilities. Falls back to the static
/// list when the response has no models.
fn parse_anthropic_models(data: &Value) -> Vec<ModelInfo> {
    let Some(arr) = data.get("data").and_then(|d| d.as_array()) else {
        return anthropic_fallback_models();
    };
    let mut out: Vec<ModelInfo> = arr
        .iter()
        .filter_map(|m| {
            let id = m.get("id").and_then(|v| v.as_str())?.to_string();
            let name = m
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or(&id)
                .to_string();
            Some(anthropic_model_caps(&id, &name))
        })
        .collect();
    if out.is_empty() {
        return anthropic_fallback_models();
    }
    // de-dup by id, preserve order
    let mut seen = std::collections::HashSet::new();
    out.retain(|m| seen.insert(m.id.clone()));
    out
}

/// Curated capabilities for a Claude model id (context window, max output,
/// extended-thinking support, vision). Unknown ids get conservative defaults
/// (thinking off, vision on — Claude has had vision since 3.0).
#[allow(clippy::if_same_then_else)] // families share caps today but are kept
                                    // distinct for readability + future divergence as models gain new caps.
fn anthropic_model_caps(id: &str, name: &str) -> ModelInfo {
    let l = id.to_ascii_lowercase();
    let (ctx, max, thinking, vision) = if l.contains("opus-4") {
        (200_000, 32_000, true, true)
    } else if l.contains("sonnet-4") {
        (200_000, 16_000, true, true)
    } else if l.contains("haiku-4") {
        (200_000, 8_192, false, true)
    } else if l.contains("3-7-sonnet") || l.contains("3.7-sonnet") {
        (200_000, 8_192, true, true)
    } else if l.contains("3-5-sonnet") || l.contains("3.5-sonnet") {
        (200_000, 8_192, false, true)
    } else if l.contains("3-5-haiku") || l.contains("3.5-haiku") {
        (200_000, 8_192, false, true)
    } else if l.contains("3-opus") || l.contains("3.0-opus") {
        (200_000, 4_096, false, true)
    } else if l.contains("3-haiku") {
        (200_000, 4_096, false, true)
    } else {
        (200_000, 8_192, false, true)
    };
    ModelInfo {
        id: id.to_string(),
        name: name.to_string(),
        reasoning: thinking,
        context_window: ctx,
        max_tokens: max,
        thinking_levels: if thinking {
            DEFAULT_THINKING_LEVELS
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            Vec::new()
        },
        vision,
        ..Default::default()
    }
}

/// Static Claude model list used when `/v1/models` is unreachable.
fn anthropic_fallback_models() -> Vec<ModelInfo> {
    let ids = [
        "claude-opus-4-1",
        "claude-sonnet-4-5",
        "claude-sonnet-4-0",
        "claude-haiku-4-5",
        "claude-3-7-sonnet-20250219",
        "claude-3-5-sonnet-20241022",
        "claude-3-5-haiku-20241022",
    ];
    ids.iter().map(|id| anthropic_model_caps(id, id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_opencode_go_matches_zen_go_path() {
        assert!(is_opencode_go("https://opencode.ai/zen/go/v1"));
        assert!(is_opencode_go("https://opencode.ai/zen/go/v1/"));
        // host must be opencode.ai AND path must include /zen/go/
        assert!(!is_opencode_go("https://opencode.ai/zen/v1"));
        assert!(!is_opencode_go("https://evil.com/zen/go/v1"));
        // a look-alike host is not mistaken for opencode.ai
        assert!(!is_opencode_go("https://opencode.ai.evil.com/zen/go/v1"));
        // not umans (must not trigger Umans-only wire fields)
        assert!(!is_umans("https://opencode.ai/zen/go/v1"));
    }

    #[test]
    fn parse_umans_usage_fields() {
        // Documented /v1/usage shape from the Umans gateway (matches
        // pi-provider-umans): concurrent_sessions = current, limits.concurrency.limit
        // = guaranteed plan ceiling.
        let v = json!({
            "limits": { "concurrency": { "limit": 8 }, "requests": { "limit": 500 } },
            "usage": { "requests_in_window": 12, "concurrent_sessions": 3 }
        });
        let u = parse_umans_usage(&v);
        assert_eq!(u.used, Some(3));
        assert_eq!(u.limit, Some(8));
    }

    #[test]
    fn parse_umans_usage_unlimited_limit() {
        // A null concurrency limit = unlimited plan → None (UI renders ∞).
        let v = json!({
            "limits": { "concurrency": { "limit": null } },
            "usage": { "concurrent_sessions": 1 }
        });
        let u = parse_umans_usage(&v);
        assert_eq!(u.used, Some(1));
        assert_eq!(u.limit, None);
    }

    #[test]
    fn parse_umans_usage_missing_fields() {
        // An empty / differently-shaped payload yields None for both (UI hides).
        let u = parse_umans_usage(&json!({}));
        assert_eq!(u.used, None);
        assert_eq!(u.limit, None);
    }

    #[test]
    fn parse_umans_usage_full_windows() {
        let v = json!({
            "plan": { "display_name": "Pro", "slug": "pro" },
            "limits": { "concurrency": { "limit": 8 }, "requests": { "limit": 500 } },
            "usage": {
                "requests_in_window": 12,
                "concurrent_sessions": 3,
                "tokens_in": 1000,
                "tokens_out": 200
            },
            "window": { "remaining_minutes": 42 }
        });
        let u = parse_umans_usage_full(&v);
        assert!(u.available);
        assert_eq!(u.plan.as_deref(), Some("Pro"));
        assert!(u.windows.iter().any(|w| w.id == "concurrency"));
        assert!(u.windows.iter().any(|w| w.id == "requests"));
        let req = u.windows.iter().find(|w| w.id == "requests").unwrap();
        assert_eq!(req.used, Some(12.0));
        assert_eq!(req.limit, Some(500.0));
        assert!(req.detail.as_deref().unwrap_or("").contains("42m"));
    }

    #[test]
    fn parse_codex_usage_primary_secondary() {
        let v = json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 42,
                    "limit_window_seconds": 18000,
                    "reset_at": 9999999999_i64
                },
                "secondary_window": {
                    "used_percent": 10,
                    "limit_window_seconds": 604800,
                    "reset_at": 9999999999_i64
                }
            }
        });
        let u = parse_codex_usage(&v);
        assert!(u.available);
        assert_eq!(u.plan.as_deref(), Some("pro"));
        assert_eq!(u.windows.len(), 2);
        assert_eq!(u.windows[0].id, "five_hour");
        assert_eq!(u.windows[0].used, Some(42.0));
        assert_eq!(u.windows[0].unit, "percent");
        assert_eq!(u.windows[1].id, "weekly");
    }

    #[test]
    fn parse_anthropic_oauth_usage_windows() {
        let v = json!({
            "five_hour": { "utilization": 72.5, "resets_at": "2026-07-09T18:00:00Z" },
            "seven_day": { "utilization": 30, "resets_at": 9999999999_i64 },
            "seven_day_opus": { "utilization": 5, "resets_at": null }
        });
        let u = parse_anthropic_oauth_usage(&v);
        assert!(u.available);
        assert_eq!(u.windows.len(), 3);
        let five = u.windows.iter().find(|w| w.id == "five_hour").unwrap();
        assert_eq!(five.used, Some(72.5));
        assert_eq!(five.unit, "percent");
        assert!(five.resets_at.is_some());
        let week = u.windows.iter().find(|w| w.id == "seven_day").unwrap();
        assert_eq!(week.used, Some(30.0));
    }

    #[test]
    fn parse_iso8601_unix_basic() {
        // 2026-07-09T00:00:00Z
        let ts = parse_iso8601_unix("2026-07-09T00:00:00Z").unwrap();
        // Sanity: after 2020 and before 2030
        assert!(ts > 1_577_836_800);
        assert!(ts < 1_893_456_000);
        assert_eq!(parse_iso8601_unix("1710000000"), Some(1710000000));
    }

    #[test]
    fn parse_xai_billing_credits_format_matches_website() {
        // Live shape from /v1/billing?format=credits — matches Settings → Usage.
        let v = json!({
            "config": {
                "currentPeriod": {
                    "type": "USAGE_PERIOD_TYPE_WEEKLY",
                    "start": "2026-07-09T14:26:33.371434+00:00",
                    "end": "2026-07-16T14:26:33.371434+00:00"
                },
                "creditUsagePercent": 30.0,
                "onDemandCap": { "val": 0 },
                "onDemandUsed": { "val": 0 },
                "productUsage": [
                    { "product": "GrokBuild", "usagePercent": 29.0 },
                    { "product": "Api", "usagePercent": 1.0 },
                    { "product": "Chat", "usagePercent": 0.0 }
                ],
                "isUnifiedBillingUser": true,
                "prepaidBalance": { "val": 0 },
                "billingPeriodStart": "2026-07-09T14:26:33.371434+00:00",
                "billingPeriodEnd": "2026-07-16T14:26:33.371434+00:00"
            }
        });
        let u = parse_xai_billing(&v);
        assert!(u.available);
        let weekly = u.windows.iter().find(|w| w.id == "weekly").unwrap();
        assert_eq!(weekly.label, "Weekly usage");
        assert_eq!(weekly.used, Some(30.0));
        assert_eq!(weekly.limit, Some(100.0));
        assert_eq!(weekly.unit, "percent");
        assert!(weekly.resets_at.is_some());
        // Product rows (zero-share Chat skipped).
        let build = u
            .windows
            .iter()
            .find(|w| w.id == "product_grokbuild")
            .unwrap();
        assert_eq!(build.label, "Build");
        assert_eq!(build.used, Some(29.0));
        let api = u.windows.iter().find(|w| w.id == "product_api").unwrap();
        assert_eq!(api.used, Some(1.0));
        assert!(!u.windows.iter().any(|w| w.id == "product_chat"));
    }

    #[test]
    fn parse_xai_billing_legacy_raw_credits_fallback() {
        // Without ?format=credits the host returns used/monthlyLimit only.
        let v = json!({
            "config": {
                "monthlyLimit": { "val": 15000 },
                "used": { "val": 885 },
                "onDemandCap": { "val": 0 },
                "billingPeriodStart": "2026-07-01T00:00:00+00:00",
                "billingPeriodEnd": "2026-08-01T00:00:00+00:00"
            }
        });
        let u = parse_xai_billing(&v);
        assert!(u.available);
        let w = u.windows.iter().find(|w| w.id == "weekly").unwrap();
        assert_eq!(w.used, Some(885.0));
        assert_eq!(w.limit, Some(15000.0));
        assert_eq!(w.unit, "credits");
    }

    #[test]
    fn parse_xai_billing_with_on_demand() {
        let v = json!({
            "config": {
                "creditUsagePercent": 100.0,
                "currentPeriod": {
                    "type": "USAGE_PERIOD_TYPE_WEEKLY",
                    "start": "2026-07-09T00:00:00Z",
                    "end": "2026-07-16T00:00:00Z"
                },
                "onDemandCap": { "val": 5000 },
                "onDemandUsed": { "val": 1200 },
                "prepaidBalance": { "val": 50 }
            }
        });
        let u = parse_xai_billing(&v);
        assert!(u.available);
        assert!(u.windows.iter().any(|w| w.id == "weekly"));
        let od = u.windows.iter().find(|w| w.id == "on_demand").unwrap();
        assert_eq!(od.used, Some(1200.0));
        assert_eq!(od.limit, Some(5000.0));
        let pre = u.windows.iter().find(|w| w.id == "prepaid").unwrap();
        assert_eq!(pre.limit, Some(50.0));
    }

    #[test]
    fn parse_xai_subscription_plan_label() {
        let v = json!({
            "subscriptions": [{
                "tier": "SUBSCRIPTION_TIER_GROK_PRO",
                "status": "SUBSCRIPTION_STATUS_ACTIVE"
            }]
        });
        assert_eq!(
            parse_xai_subscription_plan(&v).as_deref(),
            Some("SuperGrok")
        );
    }

    #[test]
    fn opencode_go_curated_lists_partition_by_protocol() {
        let openai = opencode_go_openai_models();
        let anth = opencode_go_anthropic_models();
        // the OpenCode Go docs map exactly these models to each protocol
        assert_eq!(
            openai.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
            vec![
                "glm-5.2",
                "glm-5.1",
                "kimi-k2.7-code",
                "kimi-k2.6",
                "deepseek-v4-pro",
                "deepseek-v4-flash",
                "mimo-v2.5",
                "mimo-v2.5-pro",
            ]
        );
        assert_eq!(
            anth.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
            vec![
                "minimax-m3",
                "minimax-m2.7",
                "minimax-m2.5",
                "qwen3.7-max",
                "qwen3.7-plus",
                "qwen3.6-plus",
            ]
        );
        // no model appears in both lists (each routes to exactly one protocol)
        let mut all: Vec<String> = openai
            .iter()
            .chain(anth.iter())
            .map(|m| m.id.clone())
            .collect();
        all.sort();
        let mut deduped = all.clone();
        deduped.dedup();
        assert_eq!(
            all.len(),
            deduped.len(),
            "model id duplicated across protocols"
        );
        // conservative, honest capabilities: no advertised thinking levels (so
        // no reasoning_effort/thinking block is ever sent over this endpoint)
        // OpenAI-served models: no reasoning (reasoning_effort is Umans-only)
        for m in &openai {
            assert!(
                m.thinking_levels.is_empty(),
                "OpenAI {} has thinking levels",
                m.id
            );
            assert!(!m.reasoning, "OpenAI {} marked reasoning", m.id);
            assert!(m.context_window > 0 && m.max_tokens > 0);
        }
        // Anthropic-served models: extended thinking enabled
        for m in &anth {
            assert!(
                !m.thinking_levels.is_empty(),
                "Anthropic {} has no thinking levels",
                m.id
            );
            assert!(m.reasoning, "Anthropic {} not marked reasoning", m.id);
            assert!(m.context_window > 0 && m.max_tokens > 0);
        }
    }

    #[test]
    fn opencode_go_model_protocol_partitions_by_family() {
        // OpenAI chat/completions families (incl. ids the docs table hasn't
        // caught up to).
        for id in [
            "glm-5.2",
            "glm-5",
            "kimi-k2.7-code",
            "kimi-k2.5",
            "deepseek-v4-pro",
            "mimo-v2.5",
            "mimo-v2-omni",
        ] {
            assert_eq!(
                opencode_go_model_protocol(id),
                Some(true),
                "{id} should be OpenAI"
            );
        }
        // Anthropic /v1/messages families.
        for id in [
            "minimax-m3",
            "minimax-m2.7",
            "qwen3.7-max",
            "qwen3.5-plus",
            "qwen3.6-plus",
        ] {
            assert_eq!(
                opencode_go_model_protocol(id),
                Some(false),
                "{id} should be Anthropic"
            );
        }
        // Unknown family → None (dropped, not misrouted).
        assert_eq!(opencode_go_model_protocol("hy3-preview"), None);
    }

    #[test]
    fn opencode_go_filter_models_partitions_live_endpoint_payload() {
        // Shape returned by https://opencode.ai/zen/go/v1/models (OpenAI-style
        // {data:[{id,...}]}; no display name, no protocol field). Includes ids
        // beyond the docs table (kimi-k2.5, glm-5, qwen3.5-plus, mimo-v2-pro,
        // mimo-v2-omni) and one unknown-family id (hy3-preview).
        let payload = json!({
            "object": "list",
            "data": [
                {"id":"minimax-m3","object":"model","owned_by":"opencode"},
                {"id":"minimax-m2.7","object":"model","owned_by":"opencode"},
                {"id":"minimax-m2.5","object":"model","owned_by":"opencode"},
                {"id":"kimi-k2.7-code","object":"model","owned_by":"opencode"},
                {"id":"kimi-k2.6","object":"model","owned_by":"opencode"},
                {"id":"kimi-k2.5","object":"model","owned_by":"opencode"},
                {"id":"glm-5.2","object":"model","owned_by":"opencode"},
                {"id":"glm-5.1","object":"model","owned_by":"opencode"},
                {"id":"glm-5","object":"model","owned_by":"opencode"},
                {"id":"deepseek-v4-pro","object":"model","owned_by":"opencode"},
                {"id":"deepseek-v4-flash","object":"model","owned_by":"opencode"},
                {"id":"qwen3.7-max","object":"model","owned_by":"opencode"},
                {"id":"qwen3.7-plus","object":"model","owned_by":"opencode"},
                {"id":"qwen3.6-plus","object":"model","owned_by":"opencode"},
                {"id":"qwen3.5-plus","object":"model","owned_by":"opencode"},
                {"id":"mimo-v2-pro","object":"model","owned_by":"opencode"},
                {"id":"mimo-v2-omni","object":"model","owned_by":"opencode"},
                {"id":"mimo-v2.5-pro","object":"model","owned_by":"opencode"},
                {"id":"mimo-v2.5","object":"model","owned_by":"opencode"},
                {"id":"hy3-preview","object":"model","owned_by":"opencode"}
            ]
        });
        let openai = opencode_go_filter_models(&payload, true);
        let anth = opencode_go_filter_models(&payload, false);
        // OpenAI partition: glm/kimi/deepseek/mimo families (order preserved).
        assert_eq!(
            openai.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
            vec![
                "kimi-k2.7-code",
                "kimi-k2.6",
                "kimi-k2.5",
                "glm-5.2",
                "glm-5.1",
                "glm-5",
                "deepseek-v4-pro",
                "deepseek-v4-flash",
                "mimo-v2-pro",
                "mimo-v2-omni",
                "mimo-v2.5-pro",
                "mimo-v2.5",
            ]
        );
        // Anthropic partition: minimax/qwen families.
        assert_eq!(
            anth.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
            vec![
                "minimax-m3",
                "minimax-m2.7",
                "minimax-m2.5",
                "qwen3.7-max",
                "qwen3.7-plus",
                "qwen3.6-plus",
                "qwen3.5-plus",
            ]
        );
        // No overlap between partitions.
        let mut all: Vec<String> = openai
            .iter()
            .chain(anth.iter())
            .map(|m| m.id.clone())
            .collect();
        all.sort();
        let mut deduped = all.clone();
        deduped.dedup();
        assert_eq!(all.len(), deduped.len(), "id in both partitions");
        // hy3-preview (unknown family) is dropped, not misrouted.
        assert!(!openai.iter().any(|m| m.id == "hy3-preview"));
        assert!(!anth.iter().any(|m| m.id == "hy3-preview"));
        // Known ids keep their curated display name; new ids get a synthesized one.
        assert_eq!(
            openai.iter().find(|m| m.id == "glm-5.2").unwrap().name,
            "GLM-5.2"
        );
        assert_eq!(
            openai.iter().find(|m| m.id == "kimi-k2.5").unwrap().name,
            "Kimi K2.5"
        );
        assert_eq!(
            anth.iter().find(|m| m.id == "qwen3.5-plus").unwrap().name,
            "Qwen 3.5 Plus"
        );
        // Capabilities: OpenAI-served no reasoning; Anthropic-served have thinking.
        for m in &openai {
            assert!(
                m.thinking_levels.is_empty(),
                "OpenAI {} has thinking levels",
                m.id
            );
            assert!(!m.reasoning, "OpenAI {} marked reasoning", m.id);
        }
        for m in &anth {
            assert!(
                !m.thinking_levels.is_empty(),
                "Anthropic {} has no thinking levels",
                m.id
            );
            assert!(m.reasoning, "Anthropic {} not marked reasoning", m.id);
        }
        // Malformed payload → empty (no panic).
        assert!(opencode_go_filter_models(&json!({}), true).is_empty());
        assert!(opencode_go_filter_models(&json!({"data":[]}), true).is_empty());
    }

    #[test]
    fn opencode_go_display_name_synthesizes_unknown_ids() {
        // Known → curated exact name.
        assert_eq!(opencode_go_display_name("glm-5.2"), "GLM-5.2");
        assert_eq!(opencode_go_display_name("kimi-k2.7-code"), "Kimi K2.7 Code");
        assert_eq!(opencode_go_display_name("qwen3.7-max"), "Qwen3.7 Max");
        // Unknown → synthesized "Brand <Rest>".
        assert_eq!(opencode_go_display_name("kimi-k2.5"), "Kimi K2.5");
        assert_eq!(opencode_go_display_name("glm-5"), "GLM 5");
        assert_eq!(opencode_go_display_name("qwen3.5-plus"), "Qwen 3.5 Plus");
        assert_eq!(opencode_go_display_name("mimo-v2-omni"), "MiMo V2 Omni");
        // Totally unknown family → raw id.
        assert_eq!(opencode_go_display_name("hy3-preview"), "hy3-preview");
    }

    #[test]
    fn token_count_handles_int_float_and_string() {
        // integer (standard OpenAI)
        assert_eq!(token_count(&json!(1234)), Some(1234));
        // float — some proxies serialize counts as `100.0`
        assert_eq!(token_count(&json!(100.0)), Some(100));
        // quoted number
        assert_eq!(token_count(&json!("567")), Some(567));
        // absent / null / garbage
        assert_eq!(token_count(&Value::Null), None);
        assert_eq!(token_count(&json!("n/a")), None);
    }

    #[test]
    fn parse_http_date_known_epochs() {
        // P2-6: HTTP-date Retry-After parsing.
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        // 2025-01-01 00:00:00 UTC = 1735689600
        assert_eq!(
            parse_http_date("Wed, 01 Jan 2025 00:00:00 GMT"),
            Some(1735689600)
        );
        // weekday is ignored (servers sometimes send the wrong one)
        assert_eq!(
            parse_http_date("Mon, 01 Jan 2025 00:00:00 GMT"),
            Some(1735689600)
        );
    }

    #[test]
    fn parse_retry_after_int_seconds() {
        assert_eq!(parse_retry_after("5"), Some(5));
        assert_eq!(parse_retry_after("  10 "), Some(10));
        assert!(parse_retry_after("garbage").is_none());
    }

    #[test]
    fn sanitize_inserts_synthetic_results() {
        let mut msgs: Vec<Message> = vec![
            Message::user("hi"),
            Message::assistant_tool_calls(vec![crate::message::ToolCall {
                id: "call_1".into(),
                typ: "function".into(),
                function: crate::message::FunctionCall {
                    name: "bash".into(),
                    arguments: "{}".into(),
                },
            }]),
        ];
        let n = sanitize_orphaned_tool_calls(&mut msgs);
        // a tool result for call_1 should now follow the assistant message
        let has_result = msgs
            .iter()
            .any(|m| m.is_tool() && m.tool_call_id() == Some("call_1"));
        assert!(has_result);
        assert_eq!(msgs.len(), 3);
        assert_eq!(n, 1, "should report 1 synthetic result inserted");
    }

    #[test]
    fn sanitize_drops_orphaned_results() {
        // Compaction kept a `tool` result whose matching assistant `tool_calls`
        // was dropped. The orphaned `tool` message must be removed (not left to
        // 400 the request), and no synthetic call is inserted (there's no call
        // to synthesize a result for).
        let mut msgs: Vec<Message> = vec![
            Message::user("hi"),
            Message::tool("ghost_call", "stale result"),
            Message::assistant("ok"),
        ];
        let n = sanitize_orphaned_tool_calls(&mut msgs);
        assert!(
            !msgs.iter().any(|m| m.is_tool()),
            "orphaned tool result should be dropped: {msgs:?}"
        );
        assert_eq!(msgs.len(), 2);
        assert_eq!(n, 1, "should report 1 orphaned result dropped");
    }

    #[test]
    fn sanitize_noop_when_results_present() {
        let mut msgs: Vec<Message> = vec![
            Message::assistant_tool_calls(vec![crate::message::ToolCall {
                id: "c1".into(),
                typ: "function".into(),
                function: crate::message::FunctionCall {
                    name: "x".into(),
                    arguments: "{}".into(),
                },
            }]),
            Message::tool("c1", "ok"),
        ];
        let n = sanitize_orphaned_tool_calls(&mut msgs);
        assert_eq!(msgs.len(), 2);
        assert_eq!(n, 0, "clean conversation: no fixes");
    }

    #[test]
    fn sanitize_args_fixes_malformed_arguments() {
        let mut msgs: Vec<Message> = vec![
            Message::assistant_tool_calls(vec![
                crate::message::ToolCall {
                    id: "c1".into(),
                    typ: "function".into(),
                    function: crate::message::FunctionCall {
                        name: "bulk".into(),
                        arguments: "{broken json".into(),
                    },
                },
                crate::message::ToolCall {
                    id: "c2".into(),
                    typ: "function".into(),
                    function: crate::message::FunctionCall {
                        name: "bash".into(),
                        arguments: "{\"command\":\"echo hi\"}".into(),
                    },
                },
                crate::message::ToolCall {
                    id: "c3".into(),
                    typ: "function".into(),
                    function: crate::message::FunctionCall {
                        name: "bulk".into(),
                        arguments: "{\"calls\":[{\"name\":\"bash\",\"args\":{\"command\":\"echo '"
                            .into(),
                    },
                },
            ]),
            Message::tool("c1", "err"),
            Message::tool("c2", "ok"),
            Message::tool("c3", "err"),
        ];
        let n = sanitize_tool_call_arguments(&mut msgs);
        assert_eq!(n, 2, "only the two malformed calls should be fixed");
        let calls = msgs[0].tool_calls().unwrap();
        assert_eq!(calls[0].function.arguments, "{}");
        assert_eq!(calls[1].function.arguments, "{\"command\":\"echo hi\"}");
        assert_eq!(calls[2].function.arguments, "{}");
        // every arguments field must now be valid JSON
        for tc in calls {
            serde_json::from_str::<Value>(&tc.function.arguments).unwrap();
        }
    }

    #[test]
    fn sanitize_args_coerces_non_json_arguments() {
        // A tool call with garbage arguments (not valid JSON at all)
        // gets fixed to "{}".
        let mut msgs: Vec<Message> = vec![Message::assistant_tool_calls(vec![
            crate::message::ToolCall {
                id: "c1".into(),
                typ: "function".into(),
                function: crate::message::FunctionCall {
                    name: "bash".into(),
                    arguments: "not valid json".into(),
                },
            },
        ])];
        let n = sanitize_tool_call_arguments(&mut msgs);
        assert_eq!(n, 1);
        let args = &msgs[0].tool_calls().unwrap()[0].function.arguments;
        assert_eq!(args, "{}");
    }

    #[test]
    fn sanitize_args_skips_non_assistant_messages() {
        let mut msgs: Vec<Message> = vec![
            Message::user("hi"),
            Message::tool("x", "{not real json but role is tool}"),
        ];
        assert_eq!(sanitize_tool_call_arguments(&mut msgs), 0);
    }

    #[test]
    fn backoff_progression() {
        assert_eq!(backoff_ms(1, None), 500);
        assert_eq!(backoff_ms(2, None), 1000);
        assert_eq!(backoff_ms(3, None), 2000);
        assert_eq!(backoff_ms(4, None), 4000);
        assert_eq!(backoff_ms(8, None), 8000); // capped
        assert_eq!(backoff_ms(2, Some(3)), 3000); // Retry-After honored
        assert_eq!(backoff_ms(2, Some(60)), 30000); // Retry-After capped at 30s
    }

    #[test]
    fn is_umans_detection() {
        assert!(is_umans("https://api.code.umans.ai/v1"));
        assert!(is_umans("https://umans.ai/v1"));
        assert!(!is_umans("https://api.openai.com/v1"));
        assert!(!is_umans("https://localhost:11434/v1"));
        // Look-alike host must NOT be detected (substring `.contains` false-pos):
        // `api.umans.ai.evil.com` is not a subdomain of umans.ai.
        assert!(!is_umans("https://api.umans.ai.evil.com/v1"));
        assert!(!is_umans("https://umans.ai.evil.com/v1"));
        // port suffix is handled
        assert!(is_umans("https://api.umans.ai:443/v1"));
    }

    #[test]
    fn is_xai_endpoint_detection() {
        assert!(is_xai_endpoint("https://api.x.ai/v1"));
        assert!(is_xai_endpoint("https://api.x.ai/v1/"));
        assert!(is_xai_endpoint("https://x.ai/v1"));
        assert!(!is_xai_endpoint("https://api.openai.com/v1"));
        assert!(!is_xai_endpoint("https://api.x.ai.evil.com/v1"));
    }

    #[test]
    fn parse_xai_models_list_uses_live_context_and_filters_media() {
        let data = json!({
            "data": [
                {
                    "id": "grok-4.5",
                    "context_length": 500000,
                    "completion_text_token_price": 60000,
                    "prompt_image_token_price": 20000
                },
                {
                    "id": "grok-build-0.1",
                    "context_length": 256000,
                    "completion_text_token_price": 20000,
                    "prompt_image_token_price": 10000
                },
                {
                    "id": "grok-4.20-0309-non-reasoning",
                    "context_length": 1000000,
                    "completion_text_token_price": 25000,
                    "prompt_image_token_price": 12500
                },
                {
                    "id": "grok-imagine-image",
                    "context_length": 8000,
                    "image_price": 200000000
                },
                {
                    "id": "grok-imagine-video-1.5",
                    "owned_by": "xai"
                }
            ]
        });
        let models = parse_xai_models_list(&data);
        assert_eq!(models.len(), 3, "media models filtered: {:?}", models);
        // Coding default pinned first.
        assert_eq!(models[0].id, "grok-build-0.1");
        assert_eq!(models[0].context_window, 256_000);
        assert!(models[0].reasoning);
        assert!(models[0].vision);
        assert!(!models[0].thinking_levels.is_empty());

        let g45 = models.iter().find(|m| m.id == "grok-4.5").unwrap();
        assert_eq!(g45.context_window, 500_000);
        assert!(g45.vision);

        let non = models
            .iter()
            .find(|m| m.id == "grok-4.20-0309-non-reasoning")
            .unwrap();
        assert_eq!(non.context_window, 1_000_000);
        assert!(!non.reasoning);
        assert!(non.thinking_levels.is_empty());

        // No media models.
        assert!(models.iter().all(|m| !m.id.contains("imagine")));
    }

    #[test]
    fn apply_xai_language_models_enrichment_filters_and_sets_vision() {
        let mut models = vec![
            xai_model_caps("grok-build-0.1", "grok-build-0.1"),
            xai_model_caps("mystery-not-in-lang", "mystery"),
        ];
        let mut lang = std::collections::HashMap::new();
        lang.insert("grok-build-0.1".into(), true);
        apply_xai_language_models_enrichment(&mut models, &lang);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "grok-build-0.1");
        assert!(models[0].vision);
    }

    #[test]
    fn apply_live_model_fields_overlays_context_window() {
        let mut info = openai_model_caps("unknown-model", "unknown-model");
        assert_eq!(info.context_window, 200_000); // curated default
        apply_live_model_fields(
            &json!({"context_length": 750000, "prompt_image_token_price": 1}),
            &mut info,
        );
        assert_eq!(info.context_window, 750_000);
        assert!(info.vision);
    }

    #[test]
    fn resolve_effort_passthrough_when_no_levels() {
        assert_eq!(resolve_effort("medium", &[]), "medium");
        assert_eq!(resolve_effort("banana", &[]), "banana");
    }

    #[test]
    fn resolve_effort_keeps_supported_case_insensitive() {
        let levels = vec!["Low".into(), "Medium".into(), "High".into()];
        // supported → preserved, but returns the model's own casing
        assert_eq!(resolve_effort("medium", &levels), "Medium");
        assert_eq!(resolve_effort("HIGH", &levels), "High");
    }

    #[test]
    fn resolve_effort_clamps_unsupported_to_preferred() {
        let levels = vec!["low".into(), "medium".into(), "high".into()];
        // unknown effort → prefers high, then medium, then low
        assert_eq!(resolve_effort("max", &levels), "high");
        assert_eq!(resolve_effort("turbo", &levels), "high");
    }

    #[test]
    fn resolve_effort_glm_only_high() {
        // GLM advertises only "high": anything else clamps to it.
        let levels = vec!["high".into()];
        assert_eq!(resolve_effort("medium", &levels), "high");
        assert_eq!(resolve_effort("low", &levels), "high");
        assert_eq!(resolve_effort("high", &levels), "high");
    }

    #[test]
    fn resolve_effort_custom_levels_no_high() {
        // A model that only exposes low+medium: unknown → medium (preferred).
        let levels = vec!["low".into(), "medium".into()];
        assert_eq!(resolve_effort("high", &levels), "medium");
        assert_eq!(resolve_effort("zzz", &levels), "medium");
    }

    #[test]
    fn fallback_models_advertise_levels() {
        let models = fallback_models();
        // every fallback entry has at least one thinking level
        assert!(models.iter().all(|m| !m.thinking_levels.is_empty()));
        // GLM entries advertise only "high"
        for m in models.iter().filter(|m| m.id.contains("glm")) {
            assert_eq!(m.thinking_levels, vec!["high".to_string()]);
        }
        // a non-GLM model advertises the standard trio
        let coder = models.iter().find(|m| m.id == "umans-coder").unwrap();
        assert_eq!(
            coder.thinking_levels,
            vec!["low".to_string(), "medium".to_string(), "high".to_string()]
        );
    }

    #[test]
    fn parse_models_response_reads_vision_flag() {
        // The endpoint exposes vision as capabilities.supports_vision, encoded
        // as true / false / "via-handoff". Only boolean true counts as native
        // client-side vision; "via-handoff" (vision only on /v1/messages, which
        // the harness doesn't use) falls through to false.
        let data = json!({
            "vision-model": { "display_name": "Vision", "capabilities": { "context_window": 128000, "recommended_max_tokens": 4096, "supports_vision": true } },
            "text-model": { "display_name": "Text", "capabilities": { "context_window": 128000, "recommended_max_tokens": 4096, "supports_vision": false } },
            "handoff-model": { "display_name": "Handoff", "capabilities": { "context_window": 128000, "recommended_max_tokens": 4096, "supports_vision": "via-handoff" } },
            "unspecified": { "display_name": "Unspec", "capabilities": { "context_window": 128000 } }
        });
        let models = parse_models_response(&data);
        let by_id: std::collections::HashMap<&str, &ModelInfo> =
            models.iter().map(|m| (m.id.as_str(), m)).collect();
        assert!(by_id["vision-model"].vision);
        assert!(!by_id["text-model"].vision);
        assert!(!by_id["handoff-model"].vision); // "via-handoff" is not native client-side vision
        assert!(!by_id["unspecified"].vision); // default false when absent
    }

    #[test]
    fn parse_codex_models_response_uses_subscription_catalog() {
        let data = json!({
            "models": [
                {
                    "slug": "chatgpt-remote-only",
                    "display_name": "ChatGPT Remote Only",
                    "supported_in_api": true,
                    "supported_reasoning_levels": [
                        {"effort": "max", "description": "Maximum"},
                        {"effort": "focused", "description": "Focused"}
                    ],
                    "context_window": 272000,
                    "supports_image_detail_original": true
                },
                {"slug": "hidden", "supported_in_api": false}
            ]
        });
        let models = parse_codex_models_response(&data);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "chatgpt-remote-only");
        assert_eq!(models[0].name, "ChatGPT Remote Only");
        assert_eq!(models[0].context_window, 272000);
        assert_eq!(
            models[0].thinking_levels,
            vec!["max".to_string(), "focused".to_string()]
        );
        assert!(models[0].vision);
    }

    #[test]
    fn modelinfo_vision_defaults_false_when_absent() {
        let j = r#"{"id":"x","name":"X","context_window":1,"max_tokens":1}"#;
        let m: ModelInfo = serde_json::from_str(j).unwrap();
        assert!(!m.vision);
        let j2 = r#"{"id":"x","name":"X","context_window":1,"max_tokens":1,"vision":true}"#;
        let m2: ModelInfo = serde_json::from_str(j2).unwrap();
        assert!(m2.vision);
    }

    #[test]
    fn parse_models_response_reads_reasoning_levels_nested() {
        // The live /models/info endpoint nests reasoning levels under
        // capabilities.reasoning.levels (not a flat capabilities.thinking_levels).
        let data = json!({
            "umans-glm-5.2": { "display_name": "Umans GLM 5.2", "capabilities": {
                "context_window": 405504, "recommended_max_tokens": 131071,
                "reasoning": { "supported": true, "can_disable": true, "levels": ["none","high","max"], "default_level": "high" }
            }},
            "umans-flash": { "display_name": "Umans Flash", "capabilities": {
                "context_window": 262144, "recommended_max_tokens": 32768,
                "reasoning": { "supported": true, "can_disable": true, "levels": ["none","low","medium","high"], "default_level": "medium" }
            }},
            "umans-kimi-k2.7": { "display_name": "Umans Kimi K2.7", "capabilities": {
                "context_window": 262144, "recommended_max_tokens": 32768,
                "reasoning": { "supported": true, "can_disable": false, "levels": [], "default_level": null }
            }}
        });
        let models = parse_models_response(&data);
        let by_id: std::collections::HashMap<&str, &ModelInfo> =
            models.iter().map(|m| (m.id.as_str(), m)).collect();
        assert_eq!(
            by_id["umans-glm-5.2"].thinking_levels,
            vec!["none".to_string(), "high".to_string(), "max".to_string()]
        );
        assert_eq!(
            by_id["umans-flash"].thinking_levels,
            vec![
                "none".to_string(),
                "low".to_string(),
                "medium".to_string(),
                "high".to_string()
            ]
        );
        assert!(by_id["umans-kimi-k2.7"].thinking_levels.is_empty());
        // reasoning flag follows reasoning.supported
        assert!(by_id["umans-glm-5.2"].reasoning);
        assert!(by_id["umans-kimi-k2.7"].reasoning);
    }

    #[test]
    fn parse_models_response_reasoning_supported_false() {
        let data = json!({
            "no-think": { "display_name": "No Think", "capabilities": {
                "context_window": 128000, "recommended_max_tokens": 4096,
                "reasoning": { "supported": false, "levels": [] }
            }}
        });
        let models = parse_models_response(&data);
        assert!(!models[0].reasoning);
        assert!(models[0].thinking_levels.is_empty());
    }

    #[test]
    fn parse_models_response_flat_levels_fallback() {
        // Endpoints that expose levels as a flat capability field still parse.
        let data = json!({
            "flat-model": { "display_name": "Flat", "capabilities": {
                "context_window": 128000, "recommended_max_tokens": 4096,
                "reasoning_levels": ["low","high"]
            }}
        });
        let models = parse_models_response(&data);
        assert_eq!(
            models[0].thinking_levels,
            vec!["low".to_string(), "high".to_string()]
        );
    }

    #[test]
    fn cache_version_gate() {
        // A cache with the current version is accepted.
        assert!(cache_version_ok(
            &json!({ "version": MODELS_CACHE_VERSION })
        ));
        // A pre-versioning cache (no version field) is rejected so a parser fix
        // isn't masked by stale data for the TTL window.
        assert!(!cache_version_ok(
            &json!({ "base_url": "x", "updated_at": 0 })
        ));
        // A future / mismatched version is rejected.
        assert!(!cache_version_ok(&json!({ "version": 99 })));
    }

    // ---- Anthropic translation ----

    #[test]
    fn anthropic_thinking_budget_maps_and_clamps() {
        // effort -> budget
        assert_eq!(anthropic_thinking_budget("low", 100_000), Some(4096));
        assert_eq!(anthropic_thinking_budget("medium", 100_000), Some(12288));
        assert_eq!(anthropic_thinking_budget("HIGH", 100_000), Some(24576));
        assert_eq!(anthropic_thinking_budget("max", 100_000), Some(24576));
        // unsupported effort -> no thinking
        assert_eq!(anthropic_thinking_budget("none", 100_000), None);
        assert_eq!(anthropic_thinking_budget("bogus", 100_000), None);
        // clamp to max_tokens-1024 when base exceeds it
        assert_eq!(anthropic_thinking_budget("high", 20000), Some(18976));
        // base below the cap passes through unchanged
        assert_eq!(anthropic_thinking_budget("high", 30000), Some(24576));
        // too small to leave room -> None
        assert_eq!(anthropic_thinking_budget("low", 2000), None);
        assert_eq!(anthropic_thinking_budget("high", 1500), None);
    }

    #[test]
    fn anthropic_stop_reason_maps_to_openai() {
        assert_eq!(anthropic_stop_reason("end_turn"), "stop");
        assert_eq!(anthropic_stop_reason("stop_sequence"), "stop");
        assert_eq!(anthropic_stop_reason("tool_use"), "tool_calls");
        assert_eq!(anthropic_stop_reason("max_tokens"), "length");
        assert_eq!(anthropic_stop_reason("weird"), "weird");
    }

    #[test]
    fn anthropic_image_block_data_url_and_plain_url() {
        let b = anthropic_image_block("data:image/png;base64,QUJD").unwrap();
        assert_eq!(b["type"], "image");
        assert_eq!(b["source"]["type"], "base64");
        assert_eq!(b["source"]["media_type"], "image/png");
        assert_eq!(b["source"]["data"], "QUJD");
        let b = anthropic_image_block("https://x.test/cat.png").unwrap();
        assert_eq!(b["source"]["type"], "url");
        assert_eq!(b["source"]["url"], "https://x.test/cat.png");
    }

    #[test]
    fn anthropic_content_blocks_string_and_multimodal() {
        // plain string -> single text block
        let b = anthropic_content_blocks(&json!("hi"));
        assert_eq!(b, vec![json!({ "type": "text", "text": "hi" })]);
        // multimodal: text + base64 image
        let content = json!([
            { "type": "text", "text": "look" },
            { "type": "image_url", "image_url": { "url": "data:image/jpeg;base64,ZGF0YQ==" } }
        ]);
        let b = anthropic_content_blocks(&content);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0]["type"], "text");
        assert_eq!(b[1]["type"], "image");
        assert_eq!(b[1]["source"]["media_type"], "image/jpeg");
        // empty -> placeholder text block
        let b = anthropic_content_blocks(&json!([]));
        assert_eq!(b, vec![json!({ "type": "text", "text": "" })]);
    }

    #[test]
    fn build_anthropic_extracts_system_to_toplevel() {
        let msgs = json!([
            { "role": "system", "content": "You are a coder." },
            { "role": "user", "content": "hi" }
        ]);
        let req =
            build_anthropic_request(msgs.as_array().unwrap(), &[], "claude-x", "none", &[], 4096);
        assert_eq!(req["system"], "You are a coder.");
        assert_eq!(req["model"], "claude-x");
        assert_eq!(req["max_tokens"], 4096);
        // system extracted -> messages starts with user
        assert_eq!(req["messages"][0]["role"], "user");
        assert!(req.get("tools").is_none());
        assert!(req.get("thinking").is_none());
    }

    #[test]
    fn build_anthropic_converts_tools_and_tool_choice() {
        let msgs = json!([{ "role": "user", "content": "do it" }]);
        let tools = json!([
            { "type": "function", "function": {
                "name": "read_file",
                "description": "Read a file",
                "parameters": { "type": "object", "properties": {} }
            }}
        ]);
        let req = build_anthropic_request(
            msgs.as_array().unwrap(),
            tools.as_array().unwrap(),
            "claude-x",
            "none",
            &[],
            4096,
        );
        let t = req["tools"].as_array().unwrap();
        assert_eq!(t[0]["name"], "read_file");
        assert_eq!(t[0]["description"], "Read a file");
        assert_eq!(t[0]["input_schema"]["type"], "object");
        assert_eq!(req["tool_choice"]["type"], "auto");
    }

    #[test]
    fn build_anthropic_assistant_tool_calls_become_tool_use() {
        let msgs = json!([
            { "role": "user", "content": "read foo" },
            { "role": "assistant", "content": null, "tool_calls": [
                { "id": "call_1", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"foo.rs\"}" } }
            ]},
            { "role": "tool", "tool_call_id": "call_1", "content": "contents of foo" }
        ]);
        let req =
            build_anthropic_request(msgs.as_array().unwrap(), &[], "claude-x", "none", &[], 4096);
        let m = req["messages"].as_array().unwrap();
        // user, assistant(tool_use), user(tool_result)
        assert_eq!(m.len(), 3);
        assert_eq!(m[1]["role"], "assistant");
        let blocks = m[1]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "tool_use");
        assert_eq!(blocks[0]["id"], "call_1");
        assert_eq!(blocks[0]["name"], "read_file");
        assert_eq!(blocks[0]["input"]["path"], "foo.rs");
        assert_eq!(m[2]["role"], "user");
        let rblocks = m[2]["content"].as_array().unwrap();
        assert_eq!(rblocks[0]["type"], "tool_result");
        assert_eq!(rblocks[0]["tool_use_id"], "call_1");
        assert_eq!(rblocks[0]["content"], "contents of foo");
    }

    #[test]
    fn anthropic_tool_use_start_ignores_empty_input_placeholder() {
        // Anthropic streaming: content_block_start always carries `input: {}`
        // for a tool_use; the real args arrive via input_json_delta. The start
        // handler must NOT capture that placeholder — doing so prepends "{}"
        // and corrupts the assembled args as `{}{...}` (regression observed
        // with MiniMax-M3 over the opencode-go Anthropic path).
        let mut b = AnthropicBlock::default();
        init_tool_use_block(
            &mut b,
            &json!({"type":"tool_use","id":"toolu_1","name":"bash","input":{}}),
        );
        assert_eq!(b.tool_id, "toolu_1");
        assert_eq!(b.tool_name, "bash");
        assert_eq!(b.tool_args, ""); // placeholder was NOT captured
                                     // simulate input_json_delta fragments appending the real args
        b.tool_args.push_str(r#"{"command":"ls -la"}"#);
        assert_eq!(b.tool_args, r#"{"command":"ls -la"}"#); // no leading "{}"
    }

    #[test]
    fn build_anthropic_drops_reasoning_content() {
        // Prior Umans reasoning must NOT be replayed: Anthropic rejects raw
        // thinking blocks without signatures (400). Verify it's stripped.
        let msgs = json!([
            { "role": "user", "content": "hi" },
            { "role": "assistant", "content": "hello", "reasoning_content": "secret thoughts" },
            { "role": "user", "content": "again" }
        ]);
        let req =
            build_anthropic_request(msgs.as_array().unwrap(), &[], "claude-x", "none", &[], 4096);
        let m = req["messages"].as_array().unwrap();
        let asst = &m[1];
        assert_eq!(asst["role"], "assistant");
        assert!(asst.get("reasoning_content").is_none());
        assert_eq!(asst["content"][0]["text"], "hello");
    }

    #[test]
    fn build_anthropic_merges_consecutive_same_role() {
        // Two tool results back-to-back fold into ONE user message with two
        // tool_result blocks (Anthropic requires alternating roles).
        let msgs = json!([
            { "role": "user", "content": "read two" },
            { "role": "assistant", "content": null, "tool_calls": [
                { "id": "a", "type": "function", "function": { "name": "f", "arguments": "{}" } },
                { "id": "b", "type": "function", "function": { "name": "f", "arguments": "{}" } }
            ]},
            { "role": "tool", "tool_call_id": "a", "content": "r1" },
            { "role": "tool", "tool_call_id": "b", "content": "r2" }
        ]);
        let req =
            build_anthropic_request(msgs.as_array().unwrap(), &[], "claude-x", "none", &[], 4096);
        let m = req["messages"].as_array().unwrap();
        // user, assistant, user(2 tool_results)
        assert_eq!(m.len(), 3);
        let rblocks = m[2]["content"].as_array().unwrap();
        assert_eq!(rblocks.len(), 2);
    }

    #[test]
    fn build_anthropic_enables_thinking_only_when_supported() {
        let msgs = json!([{ "role": "user", "content": "think" }]);
        // thinking-capable model advertises levels -> thinking present
        let levels: Vec<String> = vec!["low".into(), "medium".into(), "high".into()];
        let req = build_anthropic_request(
            msgs.as_array().unwrap(),
            &[],
            "claude-sonnet-4",
            "medium",
            &levels,
            100_000,
        );
        assert_eq!(req["thinking"]["type"], "enabled");
        assert_eq!(req["thinking"]["budget_tokens"], 12288);
        // non-thinking model (empty levels) -> no thinking even with effort set
        let req2 = build_anthropic_request(
            msgs.as_array().unwrap(),
            &[],
            "claude-3-5-sonnet",
            "high",
            &[],
            100_000,
        );
        assert!(req2.get("thinking").is_none());
        // effort "none" with thinking-capable -> no thinking
        let req3 = build_anthropic_request(
            msgs.as_array().unwrap(),
            &[],
            "claude-sonnet-4",
            "none",
            &levels,
            100_000,
        );
        assert!(req3.get("thinking").is_none());
    }

    #[test]
    fn anthropic_model_caps_known_families() {
        let opus = anthropic_model_caps("claude-opus-4-1-20250805", "Opus");
        assert!(opus.reasoning);
        assert!(opus.vision);
        assert_eq!(opus.max_tokens, 32_000);
        assert_eq!(opus.thinking_levels.len(), 3);
        let sonnet4 = anthropic_model_caps("claude-sonnet-4-5", "Sonnet 4.5");
        assert!(sonnet4.reasoning);
        assert_eq!(sonnet4.max_tokens, 16_000);
        let sonnet35 = anthropic_model_caps("claude-3-5-sonnet-20241022", "Sonnet 3.5");
        assert!(!sonnet35.reasoning);
        assert!(sonnet35.thinking_levels.is_empty());
        let haiku4 = anthropic_model_caps("claude-haiku-4-5", "Haiku 4.5");
        assert!(!haiku4.reasoning);
        let sonnet37 = anthropic_model_caps("claude-3-7-sonnet-20250219", "Sonnet 3.7");
        assert!(sonnet37.reasoning);
        // unknown id -> conservative defaults (no thinking, vision on)
        let unknown = anthropic_model_caps("claude-future-9", "Future");
        assert!(!unknown.reasoning);
        assert!(unknown.vision);
    }

    #[test]
    fn parse_anthropic_models_parses_and_dedups() {
        let data = json!({
            "data": [
                { "id": "claude-sonnet-4-5", "display_name": "Sonnet 4.5" },
                { "id": "claude-opus-4-1", "display_name": "Opus" },
                { "id": "claude-sonnet-4-5" }
            ],
            "has_more": false
        });
        let models = parse_anthropic_models(&data);
        assert_eq!(models.len(), 2); // dedup by id
        assert_eq!(models[0].id, "claude-sonnet-4-5");
        assert!(models[0].reasoning);
        assert_eq!(models[1].id, "claude-opus-4-1");
    }

    #[test]
    fn parse_anthropic_models_falls_back_when_empty() {
        // no data array -> static fallback list
        let models = parse_anthropic_models(&json!({}));
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id.contains("sonnet")));
        // empty data array -> fallback too
        let models = parse_anthropic_models(&json!({ "data": [] }));
        assert!(!models.is_empty());
    }

    // ---- mocked-provider integration tests ----
    // A tiny one-shot HTTP server so summarize/extract_facts exercise the real
    // reqwest HTTP path (request build, POST /chat/completions, JSON parse)
    // end-to-end against a canned OpenAI response — not just the parsers.
    fn find_header_end(b: &[u8]) -> Option<usize> {
        b.windows(4).position(|w| w == b"\r\n\r\n")
    }

    async fn mock_openai_server(response_body: String) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
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
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.flush().await.unwrap();
        });
        (base, h)
    }

    fn mock_provider(base: String) -> ResolvedProvider {
        ResolvedProvider {
            name: "mock".into(),
            kind: ProviderKind::OpenAI,
            base_url: base,
            api_key: Some("test-key".into()),
            headers: Vec::new(),
            oauth: false,
        }
    }

    #[tokio::test]
    async fn summarize_against_mock_provider() {
        let body = r#"{"choices":[{"message":{"content":"<summary>mocked</summary>"}}]}"#;
        let (base, _h) = mock_openai_server(body.into()).await;
        let client = reqwest::Client::new();
        let provider = mock_provider(base);
        let cancel = CancellationToken::new();
        let msgs: Vec<Message> = vec![
            Message::user("please refactor the auth module"),
            Message::assistant("on it"),
        ];
        let out = summarize(&client, &provider, "mock-model", &msgs, &cancel, None).await;
        assert_eq!(out.as_deref(), Some("mocked"));
    }

    #[tokio::test]
    async fn extract_facts_none_short_circuits() {
        let body = r#"{"choices":[{"message":{"content":"none"}}]}"#;
        let (base, _h) = mock_openai_server(body.into()).await;
        let client = reqwest::Client::new();
        let provider = mock_provider(base);
        let cancel = CancellationToken::new();
        let msgs: Vec<Message> = vec![Message::user("hello")];
        let out = extract_facts(&client, &provider, "mock-model", &msgs, &cancel).await;
        assert!(
            out.is_none(),
            "a 'none' reply must not be persisted as a fact"
        );
    }

    #[tokio::test]
    async fn summarize_returns_none_on_http_error() {
        let body = ""; // 200 with empty body -> JSON parse fails -> None
        let (base, _h) = mock_openai_server(body.into()).await;
        let client = reqwest::Client::new();
        let provider = mock_provider(base);
        let cancel = CancellationToken::new();
        let msgs: Vec<Message> = vec![Message::user("x")];
        let out = summarize(&client, &provider, "mock-model", &msgs, &cancel, None).await;
        assert!(out.is_none());
    }
}
