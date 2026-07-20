use crate::config::ResolvedProvider;
use crate::provider::{
    is_code_assist_endpoint, is_codex_endpoint, is_gemini_endpoint, is_opencode_go, is_umans,
    is_xai_endpoint, CLAUDE_OAUTH_BETA, CLAUDE_OAUTH_USER_AGENT, CLAUDE_OAUTH_X_APP,
};
use serde_json::{json, Value};
use std::time::Duration;

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
             Use a Claude Pro/Max subscription via a plugin OAuth provider for 5-hour and weekly limits, \
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

    let provider_message = v
        .get("message")
        .and_then(|m| m.as_str())
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string);

    let resets_at = v
        .get("window")
        .and_then(|w| w.get("resets_at"))
        .and_then(|r| {
            r.as_i64()
                .or_else(|| r.as_u64().and_then(|u| i64::try_from(u).ok()))
        });

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
            resets_at,
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
        message: provider_message,
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
pub(crate) fn parse_iso8601_unix(s: &str) -> Option<i64> {
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
        .header("anthropic-beta", CLAUDE_OAUTH_BETA)
        .header("user-agent", CLAUDE_OAUTH_USER_AGENT)
        .header("x-app", CLAUDE_OAUTH_X_APP)
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
pub(crate) fn parse_xai_subscription_plan(v: &Value) -> Option<String> {
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
