// Structured debug logging + metrics. Writes one JSON line per record to a
// debug log file (if configured) and exposes counters the core/TUI can show.
// ponytail: no tracing crate; a locked append is enough for a local harness.
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use std::time::Instant;

pub struct Logger {
    file: Mutex<Option<std::fs::File>>,
    turns: std::sync::atomic::AtomicU64,
}

#[derive(Default, Clone, Debug)]
pub struct TurnMetrics {
    pub ttft_ms: Option<u64>,
    pub elapsed_ms: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cached_tokens: u64,
    pub tps: Option<f64>,
    pub model: String,
}

impl Logger {
    pub fn new(path: Option<&std::path::Path>) -> Self {
        let file = path.and_then(|p| OpenOptions::new().create(true).append(true).open(p).ok());
        Self {
            file: Mutex::new(file),
            turns: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Number of completed turns this session (incremented by main on turn_done).
    pub fn turn_count(&self) -> u64 {
        self.turns.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn record_turn(&self) {
        self.turns.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Append one structured record. `kind` is a short tag (e.g. "tool", "http_retry").
    pub fn log(&self, kind: &str, payload: Value) {
        let rec = json!({
            "ts": now_iso(),
            "kind": kind,
            "payload": payload,
        });
        let mut line = serde_json::to_string(&rec).unwrap_or_default();
        line.push('\n');
        if let Some(f) = self.file.lock().unwrap().as_mut() {
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
        }
    }
}

/// Rough token estimate: ~4 chars per token. Good enough for compaction triggers.
/// ponytail: no tokenizer dep; this is within ~15% for code/prose, fine for a threshold.
pub fn estimate_tokens(text: &str) -> u64 {
    let chars = text.chars().count() as u64;
    chars.saturating_add(3) / 4
}

/// Estimate tokens for a whole message list (serialize each message's text fields).
pub fn estimate_messages_tokens(messages: &[Value]) -> u64 {
    let mut total = 0u64;
    for m in messages {
        total += estimate_message_tokens(m);
    }
    total
}

/// Estimate tokens for a single message. Text-only messages serialize to
/// JSON and count chars/4 (within ~15% for prose/code). Multimodal messages
/// (content is an array of parts) exclude `image_url` data URLs: a base64
/// image is ~1.4M chars but only ~1-2k model tokens, so counting its chars
/// would over-estimate by orders of magnitude and trip compaction every turn
/// for vision users. Image parts are charged a fixed per-image token cost
/// instead; text parts are estimated normally.
pub fn estimate_message_tokens(m: &Value) -> u64 {
    const PER_IMAGE_TOKENS: u64 = 768;
    if let Some(arr) = m.get("content").and_then(|v| v.as_array()) {
        let mut total = 0u64;
        let mut images = 0u64;
        for part in arr {
            match part.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "image_url" => images += 1,
                "text" => {
                    if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                        total += estimate_tokens(t);
                    }
                }
                _ => total += estimate_tokens(&serde_json::to_string(part).unwrap_or_default()),
            }
        }
        return total + images * PER_IMAGE_TOKENS + 4;
    }
    // Text-only message (content is a string): whole-message char/4 — the
    // common path, unchanged from before.
    let s = serde_json::to_string(m).unwrap_or_default();
    estimate_tokens(&s)
}

pub fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // ponytail: ISO formatting without chrono — seconds-since-epoch + a Z tag.
    format!("{secs}Z")
}

/// Helper for TTFT: record the first-token time relative to a turn start.
pub struct TurnTimer {
    pub start: Instant,
    /// Turn-level first generated token (reasoning or content). Drives TTFT
    /// (time to first token of the whole turn).
    pub first_token: Option<Instant>,
    /// First generated token of the *current* stream call. `end_call` resets it
    /// so each request is timed independently of the wall time spent waiting for
    /// tool calls to run between requests.
    pub call_first_token: Option<Instant>,
    /// Accumulated generation time across every stream call in the turn (ms):
    /// each call's first-token → end window, summed. Excludes prefill (TTFT)
    /// and tool-call wait, so TPS reflects pure model generation throughput.
    pub gen_ms: u64,
    /// Accumulated real output tokens (completion_tokens) across all stream calls.
    pub out_tokens: u64,
    /// Accumulated char/4-estimated output tokens; fallback numerator when the
    /// endpoint omits usage (out_tokens stays 0).
    pub out_tokens_est: u64,
}

impl TurnTimer {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            first_token: None,
            call_first_token: None,
            gen_ms: 0,
            out_tokens: 0,
            out_tokens_est: 0,
        }
    }
    /// Record the first generated token of the turn (TTFT) and of the current
    /// stream call (generation-time accounting). Called on the first reasoning
    /// or content chunk of each call.
    pub fn mark_first_token(&mut self) {
        let now = Instant::now();
        if self.first_token.is_none() {
            self.first_token = Some(now);
        }
        if self.call_first_token.is_none() {
            self.call_first_token = Some(now);
        }
    }
    /// Close out one stream call: fold its generation time and output tokens
    /// into the turn totals. `tokens_out` is the real completion_tokens from
    /// usage (0 when the endpoint omits usage); `est_out` is the char/4 estimate
    /// of content+reasoning, used as a fallback numerator.
    pub fn end_call(&mut self, tokens_out: u64, est_out: u64) {
        if let Some(ft) = self.call_first_token {
            self.gen_ms = self.gen_ms.saturating_add(ft.elapsed().as_millis() as u64);
        }
        self.out_tokens = self.out_tokens.saturating_add(tokens_out);
        self.out_tokens_est = self.out_tokens_est.saturating_add(est_out);
        self.call_first_token = None;
    }
    pub fn finalize(
        self,
        tokens_in: u64,
        tokens_out: u64,
        cached_tokens: u64,
        model: String,
    ) -> TurnMetrics {
        let elapsed_ms = self.start.elapsed().as_millis() as u64;
        let ttft_ms = self
            .first_token
            .map(|t| t.duration_since(self.start).as_millis() as u64);
        // TPS = output tokens / generation time. Generation time is the sum of
        // each stream call's first-token→end window, so it excludes both the
        // prefill latency (TTFT) and the wall time spent waiting for tool calls
        // to run between requests — i.e. pure model throughput, not end-to-end
        // wall time. Use real usage when the endpoint reports it; fall back to
        // the char/4 estimate otherwise. `tokens_out` (the last call's count,
        // with the reported_tokens fallback) is kept for the metrics display;
        // the TPS uses the accumulated turn-wide totals.
        let tps = if self.gen_ms > 0 {
            let n = if self.out_tokens > 0 {
                self.out_tokens
            } else {
                self.out_tokens_est
            };
            if n > 0 {
                Some(n as f64 / (self.gen_ms as f64 / 1000.0))
            } else {
                None
            }
        } else {
            None
        };
        TurnMetrics {
            ttft_ms,
            elapsed_ms,
            tokens_in,
            tokens_out,
            cached_tokens,
            tps,
            model,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn estimate_is_reasonable() {
        assert!(estimate_tokens("hello world foo bar") > 0);
        let m = vec![json!({"role":"user","content":"hello world"})];
        assert!(estimate_messages_tokens(&m) > 0);
    }

    #[test]
    fn tps_uses_generation_time_not_wall_time() {
        let mut t = TurnTimer::new();
        // Two stream calls: 500 tok / 5s + 300 tok / 3s = 800 tok / 8s = 100 tok/s.
        // The tool-call wait between the calls is NOT in gen_ms, so TPS must be
        // 100 tok/s — not 800 / total-turn-wall-time, which the old elapsed-based
        // formula computed (dumping tool-wait into the denominator).
        t.gen_ms = 8000;
        t.out_tokens = 800;
        let m = t.finalize(1000, 800, 0, "test".into());
        assert!((m.tps.unwrap() - 100.0).abs() < 0.5, "tps was {:?}", m.tps);
    }

    #[test]
    fn tps_falls_back_to_estimate_when_no_usage() {
        let mut t = TurnTimer::new();
        // Endpoint reported no usage (out_tokens 0); fall back to the char/4 estimate.
        t.gen_ms = 4000;
        t.out_tokens = 0;
        t.out_tokens_est = 400;
        let m = t.finalize(1000, 0, 0, "test".into());
        assert!((m.tps.unwrap() - 100.0).abs() < 0.5, "tps was {:?}", m.tps);
    }

    #[test]
    fn tps_none_when_no_generation_time() {
        let t = TurnTimer::new();
        // Nothing streamed into gen_ms (e.g. only untimed tool-call args) → no TPS.
        let m = t.finalize(1000, 50, 0, "test".into());
        assert!(m.tps.is_none());
    }
}
