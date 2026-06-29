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
        let file = path.and_then(|p| {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .ok()
        });
        Self { file: Mutex::new(file), turns: std::sync::atomic::AtomicU64::new(0) }
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

/// Estimate tokens for a single message (serialize to JSON, count chars/4).
pub fn estimate_message_tokens(m: &Value) -> u64 {
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
    pub first_token: Option<Instant>,
}

impl TurnTimer {
    pub fn new() -> Self {
        Self { start: Instant::now(), first_token: None }
    }
    pub fn mark_first_token(&mut self) {
        if self.first_token.is_none() {
            self.first_token = Some(Instant::now());
        }
    }
    pub fn finalize(self, tokens_in: u64, tokens_out: u64, cached_tokens: u64, model: String) -> TurnMetrics {
        let elapsed_ms = self.start.elapsed().as_millis() as u64;
        let ttft_ms = self.first_token.map(|t| t.duration_since(self.start).as_millis() as u64);
        let tps = if elapsed_ms > 0 && tokens_out > 0 {
            Some((tokens_out as f64) / (elapsed_ms as f64 / 1000.0))
        } else {
            None
        };
        TurnMetrics { ttft_ms, elapsed_ms, tokens_in, tokens_out, cached_tokens, tps, model }
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
}
