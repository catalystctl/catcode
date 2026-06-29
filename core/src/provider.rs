// OpenAI-compatible chat completions client with native Umans defaults.
// Streams SSE chunks; emits delta/thinking/tool_call events. Retries on
// transient HTTP errors with exponential backoff (honors Retry-After).
use crate::config::Config;
use crate::logging::{TurnTimer, estimate_messages_tokens, estimate_tokens};
use crate::protocol::{emit, Event, ModelInfo};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

pub const DEFAULT_BASE_URL: &str = "https://api.code.umans.ai/v1";
const MODELS_INFO_PATH: &str = "/models/info";
const CHAT_PATH: &str = "/chat/completions";

/// True if the base URL points at an Umans endpoint. Umans accepts extra
/// fields (reasoning_effort, reasoning_content replay) that vanilla OpenAI
/// servers reject with a 400 — gate those on this check.
pub fn is_umans(base_url: &str) -> bool {
    base_url.contains("umans.ai")
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

/// Summarize a slice of messages into one system message. Used by context
/// compaction so dropped turns become a short recap instead of vanishing.
/// Non-streaming, cheap; returns None on any failure (caller keeps the
/// naive drop-oldest fallback).
pub async fn summarize(
    client: &reqwest::Client,
    cfg: &Config,
    api_key: &str,
    model: &str,
    messages: &[Value],
    cancel: &CancellationToken,
) -> Option<String> {
    let body = json!({
        "model": model,
        "stream": false,
        "messages": [
            { "role": "system", "content": "Summarize the following conversation turns in structured format. Preserve: decisions made, file paths touched, the user's goal, and any unresolved errors.\n\nUse this exact format:\n<summary>\n 1. Primary Request and Intent\n 2. Key Technical Concepts\n 3. Files and Code Sections\n 4. Errors and Fixes\n 5. Problem Solving\n 6. All User Messages\n 7. Pending Tasks\n 8. Current Work\n 9. Optional Next Step\n</summary>" },
            { "role": "user", "content": messages.iter().map(|m| serde_json::to_string(m).unwrap_or_default()).collect::<Vec<_>>().join("\n") }
        ]
    });
    let url = format!("{}{CHAT_PATH}", cfg.base_url);
    let req = client.post(&url).bearer_auth(api_key).json(&body).timeout(Duration::from_secs(120));
    let resp = tokio::select! {
        r = req.send() => r.ok()?,
        _ = cancel.cancelled() => return None,
    };
    if !resp.status().is_success() { return None; }
    let v: Value = resp.json().await.ok()?;
    v.get("choices").and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
}

fn fallback_models() -> Vec<ModelInfo> {
    // ponytail: GLM chat template maps any effort except 'high' to 'max', which
    // degenerates thinking output. Advertise only ["high"] so resolve_effort
    // clamps to it — replacing the old hardcoded model-name sniff.
    let std = || DEFAULT_THINKING_LEVELS.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    vec![
        ModelInfo { id: "umans-coder".into(), name: "Umans Coder".into(), reasoning: true, context_window: 262144, max_tokens: 32768, thinking_levels: std() },
        ModelInfo { id: "umans-kimi-k2.5".into(), name: "Umans Kimi K2.5".into(), reasoning: true, context_window: 262144, max_tokens: 32768, thinking_levels: std() },
        ModelInfo { id: "umans-kimi-k2.6".into(), name: "Umans Kimi K2.6".into(), reasoning: true, context_window: 262144, max_tokens: 32768, thinking_levels: std() },
        ModelInfo { id: "umans-glm-5.1".into(), name: "Umans GLM 5.1".into(), reasoning: true, context_window: 202752, max_tokens: 131072, thinking_levels: vec!["high".to_string()] },
        ModelInfo { id: "umans-glm-5.2".into(), name: "Umans GLM 5.2".into(), reasoning: true, context_window: 413696, max_tokens: 131072, thinking_levels: vec!["high".to_string()] },
        ModelInfo { id: "umans-minimax-m2.5".into(), name: "Umans MiniMax M2.5".into(), reasoning: true, context_window: 204800, max_tokens: 8192, thinking_levels: std() },
    ]
}

/// Discover models live from /models/info; cache to disk with an 8-hour TTL.
/// Falls back to the disk cache (even if stale) on HTTP error, then to the
/// hardcoded snapshot as a last resort.
pub async fn discover_models(client: &reqwest::Client, base_url: &str) -> Vec<ModelInfo> {
    // 1. Try disk cache (fresh: < 8 hours old).
    if let Some(models) = read_models_cache(base_url) {
        return models;
    }

    // 2. Fetch live from the endpoint.
    let url = format!("{base_url}{MODELS_INFO_PATH}");
    let live = match client.get(&url).timeout(Duration::from_secs(5)).send().await {
        Ok(r) if r.status().is_success() => parse_models_response(&match r.json::<Value>().await {
            Ok(v) => v,
            Err(_) => {
                // HTTP ok but JSON parse failed — fall back to stale cache.
                return read_models_cache_stale(base_url).unwrap_or_else(fallback_models);
            }
        }),
        _ => {
            // HTTP failed — fall back to stale cache.
            return read_models_cache_stale(base_url).unwrap_or_else(fallback_models);
        }
    };

    // 3. Write fresh data to disk cache.
    write_models_cache(base_url, &live);
    live
}

/// Model cache TTL in seconds (8 hours).
const MODELS_CACHE_TTL: u64 = 28800;

fn models_cache_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(std::path::PathBuf::from(home).join(".config/umans-harness/models-cache.json"))
}

fn read_models_cache(base_url: &str) -> Option<Vec<ModelInfo>> {
    let path = models_cache_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let cache: Value = serde_json::from_str(&content).ok()?;
    let cached_url = cache.get("base_url")?.as_str()?;
    if cached_url != base_url {
        return None;
    }
    let updated = cache.get("updated_at")?.as_u64()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    if now.saturating_sub(updated) > MODELS_CACHE_TTL {
        return None;
    }
    parse_cache_models(&cache)
}

fn read_models_cache_stale(base_url: &str) -> Option<Vec<ModelInfo>> {
    let path = models_cache_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let cache: Value = serde_json::from_str(&content).ok()?;
    let cached_url = cache.get("base_url")?.as_str()?;
    if cached_url != base_url {
        return None;
    }
    parse_cache_models(&cache)
}

fn write_models_cache(base_url: &str, models: &[ModelInfo]) {
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
    let models_json: Vec<Value> = models.iter().map(|m| {
        json!({
            "id": m.id,
            "name": m.name,
            "context_window": m.context_window,
            "max_tokens": m.max_tokens,
            "thinking_levels": m.thinking_levels,
        })
    }).collect();
    let cache = json!({
        "base_url": base_url,
        "updated_at": now,
        "models": models_json,
    });
    let _ = std::fs::write(&path, serde_json::to_string(&cache).unwrap_or_default());
}

fn parse_cache_models(cache: &Value) -> Option<Vec<ModelInfo>> {
    let arr = cache.get("models")?.as_array()?;
    let mut out = Vec::new();
    for m in arr {
        let id = m.get("id")?.as_str()?.to_string();
        let name = m.get("name")?.as_str()?.to_string();
        let context_window = m.get("context_window")?.as_u64()? as u32;
        let max_tokens = m.get("max_tokens")?.as_u64()? as u32;
        let thinking_levels = m.get("thinking_levels")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        out.push(ModelInfo { id, name, reasoning: true, context_window, max_tokens, thinking_levels });
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Parse the live /models/info response into ModelInfo vec.
fn parse_models_response(data: &Value) -> Vec<ModelInfo> {
    let mut out = Vec::new();
    if let Some(obj) = data.as_object() {
        for (id, info) in obj {
            let caps = info.get("capabilities");
            let cw = caps.and_then(|c| c.get("context_window")).and_then(|v| v.as_u64()).unwrap_or(200_000) as u32;
            let mt = caps.and_then(|c| c.get("recommended_max_tokens")).and_then(|v| v.as_u64()).unwrap_or(65000) as u32;
            let name = info.get("display_name").and_then(|v| v.as_str()).unwrap_or(id).to_string();
            let thinking_levels = caps
                .and_then(|c| c.get("thinking_levels").or_else(|| c.get("reasoning_levels")).or_else(|| c.get("reasoning_efforts")))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            out.push(ModelInfo { id: id.clone(), name, reasoning: true, context_window: cw, max_tokens: mt, thinking_levels });
        }
    }
    if out.is_empty() { fallback_models() } else { out }
}

/// Sanitize orphaned tool_calls: ensure every tool_calls entry has a matching
/// tool result message. Context compaction can drop tool results while keeping
/// the assistant message that made the call, causing a 400. Mirrors the Umans
/// extension's before_provider_request handler.
/// Also verifies that the sanitizer doesn't leave behind a broken conversation
/// (validate that every assistant with tool_calls has corresponding tool results).
pub fn sanitize_orphaned_tool_calls(messages: &mut Vec<Value>) {
    let tool_call_ids: Vec<String> = messages
        .iter()
        .filter_map(|m| {
            if m.get("role").and_then(|v| v.as_str()) == Some("assistant") {
                m.get("tool_calls").and_then(|v| v.as_array())
            } else {
                None
            }
        })
        .flatten()
        .filter_map(|tc| tc.get("id").and_then(|v| v.as_str()).map(String::from))
        .collect();

    let result_ids: std::collections::HashSet<String> = messages
        .iter()
        .filter_map(|m| {
            if m.get("role").and_then(|v| v.as_str()) == Some("tool") {
                m.get("tool_call_id").and_then(|v| v.as_str()).map(String::from)
            } else {
                None
            }
        })
        .collect();

    let orphaned: Vec<String> = tool_call_ids
        .into_iter()
        .filter(|id| !result_ids.contains(id))
        .collect();
    if orphaned.is_empty() {
        return;
    }

    // Insert synthetic tool results right after the assistant message that made each call.
    let mut i = 0;
    while i < messages.len() {
        let is_assistant_with_calls = messages[i].get("role").and_then(|v| v.as_str()) == Some("assistant")
            && messages[i].get("tool_calls").and_then(|v| v.as_array()).is_some();
        if !is_assistant_with_calls {
            i += 1;
            continue;
        }
        let calls: Vec<String> = messages[i]
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .unwrap()
            .iter()
            .filter_map(|tc| tc.get("id").and_then(|v| v.as_str()).map(String::from))
            .filter(|id| orphaned.contains(id))
            .collect();
        let insert_at = i + 1;
        for (k, id) in calls.iter().enumerate() {
            messages.insert(
                insert_at + k,
                json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": "[tool result was lost during context compaction]",
                }),
            );
        }
        i = insert_at + calls.len();
    }
}

/// Read a token count from a usage field, tolerating the integer, float, and
/// string encodings different OpenAI-compatible servers emit. `as_u64` alone
/// misses floats (some proxies serialize counts as `100.0`) and quoted numbers,
/// which silently drops the context budget to zero.
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
    cfg: &Config,
    api_key: &str,
    model: &str,
    messages: &[Value],
    tools: &[Value],
    reasoning_effort: &str,
    thinking_levels: &[String],
    cancel: &CancellationToken,
    timer: &mut TurnTimer,
    quiet: bool,
) -> Result<(Value, String, u64, u64, u64), String> {
    // ponytail: reasoning_effort + reasoning_content replay are Umans-specific.
    // Only emit them when pointed at an Umans endpoint; other OpenAI-compatible
    // servers reject unknown fields with a 400.
    let base_url = &cfg.base_url;
    // ponytail: reasoning_effort + reasoning_content replay are Umans-specific.
    // Only emit them when pointed at an Umans endpoint; other OpenAI-compatible
    // servers reject unknown fields with a 400.
    let umans = is_umans(base_url);
    let mut body = json!({
        "model": model,
        "messages": messages,
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
            emit(&Event::new("info").with("message", json!(format!(
                "reasoning effort '{}' not supported by model '{}'; using '{}'",
                reasoning_effort, model, resolved
            ))));
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
    let idle = Duration::from_secs(cfg.idle_timeout_secs.max(10));

    // Live stats: estimate the prompt's token count once (the prompt is fixed for
    // this request) so the footer can show a growing context + live TPS as output
    // streams in. The real usage chunk at stream end overwrites these.
    // ponytail: char/4 estimate — same heuristic the compaction threshold uses.
    let est_prompt = estimate_messages_tokens(messages);
    let mut last_stats: Option<Instant> = None;

    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let resp = send_with_retry(client, &url, api_key, &body, cancel).await?;
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut emitted = false;
        let mut err: Option<String> = None;

        loop {
            let chunk = tokio::select! {
                c = tokio::time::timeout(idle, stream.next()) => match c {
                    Ok(x) => x,
                    Err(_) => { err = Some(format!("stream idle timeout ({}s with no data)", cfg.idle_timeout_secs)); break; }
                },
                _ = cancel.cancelled() => return Err("aborted".into()),
            };
            let Some(chunk) = chunk else { break };
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => { err = Some(format!("stream read: {}", fmt_chain(&e))); break; }
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete SSE frames. A frame may span multiple `data:` lines that
            // must be concatenated before parsing (some OpenAI-compatible servers split).
            while let Some(nl) = buf.find('\n') {
                let line = buf[..nl].trim().to_string();
                buf.drain(..=nl);
                if line.is_empty() || line.starts_with(':') {
                    continue;
                }
                let data = line
                    .strip_prefix("data: ")
                    .or_else(|| line.strip_prefix("data:"))
                    .unwrap_or("");
                if data == "[DONE]" {
                    continue;
                }
                if data.is_empty() {
                    continue;
                }
                let Ok(obj) = serde_json::from_str::<Value>(data) else { continue };

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
                    if let Some(c) = u.get("prompt_tokens_details").and_then(|d| d.get("cached_tokens")).and_then(token_count) {
                        cached_tokens = c;
                    }
                }

                let Some(choice) = obj.get("choices").and_then(|c| c.get(0)) else { continue };
                let delta = choice.get("delta");

                if let Some(c) = delta.and_then(|d| d.get("content")).and_then(|v| v.as_str()) {
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
                if let Some(r) = delta.and_then(|d| d.get("reasoning_content")).and_then(|v| v.as_str()) {
                    if !r.is_empty() {
                        reasoning.push_str(r);
                        if !quiet {
                            emitted = true;
                            emit(&Event::new("thinking").with("text", json!(r)));
                        }
                    }
                }
                if let Some(tcs) = delta.and_then(|d| d.get("tool_calls")).and_then(|v| v.as_array()) {
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
                                    emit(&Event::new("tool_call_start").with("id", json!(id)).with("index", json!(idx)));
                                }
                            }
                        }
                        let func = tc.get("function");
                        if let Some(name) = func.and_then(|f| f.get("name")).and_then(|v| v.as_str()) {
                            if acc.name.is_empty() {
                                acc.name = name.to_string();
                                if !quiet {
                                    emitted = true;
                                    emit(&Event::new("tool_call_name").with("index", json!(idx)).with("name", json!(name)));
                                }
                            }
                        }
                        if let Some(args) = func.and_then(|f| f.get("arguments")).and_then(|v| v.as_str()) {
                            acc.args.push_str(args);
                            if !quiet {
                                emitted = true;
                                emit(&Event::new("tool_call_args").with("index", json!(idx)).with("args", json!(args)));
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
            // TUI's context + TPS move during the turn, not just at the end.
            // ponytail: char/4 estimate (same heuristic as the compaction threshold);
            // the real usage chunk at stream end overwrites these with exact values.
			if !quiet && (!content.is_empty() || !reasoning.is_empty()) {
                let now = Instant::now();
                let due = last_stats.map(|t| now.duration_since(t) >= Duration::from_millis(400)).unwrap_or(true);
                if due {
                    last_stats = Some(now);
                    let est_out = estimate_tokens(&content) + estimate_tokens(&reasoning);
                    let live_ctx = est_prompt.saturating_add(est_out);
                    let mut ev = Event::new("metrics")
                        .with("tokens_in", json!(live_ctx))
                        .with("tokens_out", json!(est_out));
                    if let Some(ttft) = timer.first_token.map(|t| t.duration_since(timer.start).as_millis() as u64) {
                        ev = ev.with("ttft_ms", json!(ttft));
                    }
                    if let Some(tps) = timer.first_token.and_then(|ft| {
                        let e = ft.elapsed().as_secs_f64();
                        if e >= 0.2 { Some(est_out as f64 / e) } else { None }
                    }) {
                        ev = ev.with("tps", json!(tps));
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
        emit(&Event::new("http_retry")
            .with("attempt", json!(attempt))
            .with("reason", json!("stream error before first token"))
            .with("backoff_ms", json!(backoff)));
        // Reset accumulators for the fresh attempt.
        content.clear();
        reasoning.clear();
        tool_calls.clear();
        finish_reason.clear();
        tokens_in = 0;
        tokens_out = 0;
        cached_tokens = 0;
        sleep_or_cancel(Duration::from_millis(backoff), cancel).await?;
    }

    // Build the assistant message. OpenAI requires content null when tool_calls
    // present and empty. reasoning_content is Umans-only (gated above).
    let mut msg = serde_json::Map::new();
    msg.insert("role".into(), json!("assistant"));
    msg.insert("content".into(), if content.is_empty() { Value::Null } else { json!(content) });
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

    Ok((Value::Object(msg), finish_reason, tokens_in, tokens_out, cached_tokens))
}

/// POST with retry on 429/5xx. Exponential backoff: 0.5s, 1s, 2s, 4s (cap 8s),
/// honoring Retry-After if present. Up to 4 attempts. Cancellation-aware.
async fn send_with_retry(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
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
        let req = client
            .post(url)
            .bearer_auth(api_key)
            .json(body);

        let resp = tokio::select! {
            r = req.send() => r,
            _ = cancel.cancelled() => return Err("aborted".into()),
        };

        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                // Transport error: retry with backoff.
                if attempt >= 4 {
                    return Err(format!("request failed after {attempt} attempts: {}", fmt_chain(&e)));
                }
                let backoff = backoff_ms(attempt, None);
                emit(&Event::new("http_retry")
                    .with("attempt", json!(attempt))
                    .with("reason", json!("transport error"))
                    .with("backoff_ms", json!(backoff)));
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

        // Parse Retry-After (seconds) if present.
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());
        // Drain body before retry to free the connection.
        let _ = resp.text().await;
        let backoff = backoff_ms(attempt, retry_after);
        emit(&Event::new("http_retry")
            .with("attempt", json!(attempt))
            .with("status", json!(status.as_u16()))
            .with("backoff_ms", json!(backoff)));
        sleep_or_cancel(Duration::from_millis(backoff), cancel).await?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_inserts_synthetic_results() {
        let mut msgs = vec![
            json!({"role":"user","content":"hi"}),
            json!({"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"bash","arguments":"{}"}}]}),
        ];
        sanitize_orphaned_tool_calls(&mut msgs);
        // a tool result for call_1 should now follow the assistant message
        let has_result = msgs.iter().any(|m| {
            m.get("role").and_then(|v| v.as_str()) == Some("tool")
                && m.get("tool_call_id").and_then(|v| v.as_str()) == Some("call_1")
        });
        assert!(has_result);
        assert_eq!(msgs.len(), 3);
    }

    #[test]
    fn sanitize_noop_when_results_present() {
        let mut msgs = vec![
            json!({"role":"assistant","tool_calls":[{"id":"c1","type":"function","function":{"name":"x","arguments":"{}"}}]}),
            json!({"role":"tool","tool_call_id":"c1","content":"ok"}),
        ];
        sanitize_orphaned_tool_calls(&mut msgs);
        assert_eq!(msgs.len(), 2);
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
        assert_eq!(coder.thinking_levels, vec!["low".to_string(), "medium".to_string(), "high".to_string()]);
    }
}
