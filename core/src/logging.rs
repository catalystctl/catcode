// Structured debug logging + metrics. Writes one JSON line per record to a
// debug log file (if configured) and exposes counters the core/TUI can show.
// ponytail: no tracing crate; a locked append is enough for a local harness.
use crate::message::{Content, ContentPart, Message};
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
            // Rotate once if the debug log has grown past a generous cap, so a
            // long-running session with CATALYST_CODE_DEBUG_LOG set can't fill
            // the disk. Single rotation: the current file is renamed to <path>.1
            // (overwriting any prior .1), then a fresh file is opened below.
            // Best-effort — rename errors are ignored (we just keep appending to
            // the oversized file). Only checked on open, not on every write.
            const ROTATE_CAP: u64 = 64 * 1024 * 1024; // 64 MiB
            if let Ok(meta) = std::fs::metadata(p) {
                if meta.len() > ROTATE_CAP {
                    let mut rotated = p.as_os_str().to_os_string();
                    rotated.push(".1");
                    let _ = std::fs::rename(p, &rotated);
                }
            }
            OpenOptions::new().create(true).append(true).open(p).ok()
        });
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
    /// Restore the turn counter from a persisted session (see `session::load_stats`)
    /// so `/stats` shows the real cumulative turn count after a restart instead of
    /// resetting to zero.
    pub fn set_turns(&self, n: u64) {
        self.turns.store(n, std::sync::atomic::Ordering::SeqCst);
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
    // Byte length / 4 is ~as accurate as char/4 for ASCII-heavy code and avoids
    // a full Unicode walk on every soft-digest / compaction estimate.
    let n = text.len() as u64;
    n.saturating_add(3) / 4
}

/// Estimate tokens for a whole message list (serialize each message's text fields).
pub fn estimate_messages_tokens(messages: &[Message]) -> u64 {
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
pub fn estimate_message_tokens(m: &Message) -> u64 {
    const PER_IMAGE_TOKENS: u64 = 768;
    if let Some(parts) = m.content_parts() {
        let mut total = 0u64;
        let mut images = 0u64;
        for part in parts {
            match part {
                ContentPart::Image { .. } => images += 1,
                ContentPart::Text { text } => total += estimate_tokens(text),
            }
        }
        return total + images * PER_IMAGE_TOKENS + 4;
    }
    // Text-only message: sum role framing + text fields / 4. Avoids a full
    // serde Value round-trip on the hot soft-digest / compaction path.
    let mut n = 8u64; // role + JSON framing overhead
    match m {
        Message::System { content, .. } | Message::User { content, .. } => match content {
            Content::Text(s) => n += s.len() as u64,
            Content::Multimodal(parts) => {
                for part in parts {
                    if let ContentPart::Text { text } = part {
                        n += text.len() as u64;
                    }
                }
            }
        },
        Message::Assistant {
            content,
            thinking,
            tool_calls,
            ..
        } => {
            if let Some(c) = content {
                n += c.len() as u64;
            }
            if let Some(t) = thinking {
                n += t.len() as u64;
            }
            if let Some(tcs) = tool_calls {
                for tc in tcs {
                    n += tc.id.len() as u64;
                    n += tc.function.name.len() as u64;
                    n += tc.function.arguments.len() as u64;
                }
            }
        }
        Message::Tool {
            tool_call_id,
            content,
            ..
        } => {
            n += tool_call_id.len() as u64;
            n += content.len() as u64;
        }
    }
    n.saturating_add(3) / 4
}

/// Real-usage-anchored token estimate for a whole message list.
///
/// The endpoint reports the *real* `prompt_tokens` in its final `usage` chunk —
/// the authoritative count of the conversation exactly as the model tokenized
/// it (system prompt + every message + tool-call syntax + role framing that
/// the char/4 heuristic cannot see). When we have that number (`last_real`), use
/// it as the baseline and only char/4-estimate the messages appended *since*
/// (`len_at_real` onward). The delta is small (one assistant turn + a few tool
/// results), so its estimation error is tiny versus re-estimating the entire
/// history — which is what makes compaction fire at the right time and the
/// footer percentage track reality instead of drifting ±15-30%.
///
/// Falls back to a full `estimate_messages_tokens` when no real usage has been
/// seen yet (first turn) or right after compaction rewrites history (the old
/// baseline no longer describes the current messages).
pub fn grounded_estimate(messages: &[Message], last_real: Option<u64>, len_at_real: usize) -> u64 {
    match last_real {
        Some(real) => {
            // Clamp: a rewrite (undo/digest/compaction) may have shrunk the
            // list below the recorded index. When that happens the baseline is
            // stale and the caller should have invalidated it; clamp to len so
            // we never slice out of range, yielding just the baseline.
            let start = len_at_real.min(messages.len());
            real.saturating_add(estimate_messages_tokens(&messages[start..]))
        }
        None => estimate_messages_tokens(messages),
    }
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
    /// Turn-level first generated token (reasoning, content, or tool-call
    /// chunk). Drives TTFT (time to first token of the whole turn).
    pub first_token: Option<Instant>,
    /// First generated token of the *current* stream call. `end_call` resets it
    /// so each request is timed independently of the wall time spent waiting for
    /// tool calls to run between requests.
    pub call_first_token: Option<Instant>,
    /// Accumulated generation time across every stream call in the turn (ms):
    /// each call's first-token → end window, summed. Excludes prefill (TTFT)
    /// and tool-call wait, so TPS reflects pure model generation throughput.
    pub gen_ms: u64,
    /// Accumulated real output tokens (completion_tokens/output_tokens) across
    /// all stream calls. TPS is only reported from this real usage count; when
    /// a provider omits usage we leave TPS blank instead of showing a char/4
    /// guess as if it were measured model throughput.
    pub out_tokens: u64,
    /// Accumulated char/4-estimated output tokens. Kept for token accounting and
    /// diagnostics, but deliberately NOT used for the footer TPS widget.
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
    /// stream call (generation-time accounting). Called on the first reasoning,
    /// content, or tool-call chunk of each call.
    pub fn mark_first_token(&mut self) {
        let now = Instant::now();
        if self.first_token.is_none() {
            self.first_token = Some(now);
        }
        if self.call_first_token.is_none() {
            self.call_first_token = Some(now);
        }
    }
    /// Estimated in-flight throughput for the footer while a stream is still
    /// running. This is intentionally separate from final `tps`: the current
    /// stream's real usage is not available until the final usage chunk, so the
    /// live numerator uses the current char/4 output estimate and the UI marks it
    /// as approximate. Completed prior calls use real output tokens when any
    /// have been reported, else their estimate.
    pub fn live_tps_estimate(&self, current_est_out: u64) -> Option<f64> {
        let ft = self.call_first_token?;
        let current_ms = ft.elapsed().as_millis() as u64;
        if current_ms < 200 {
            return None;
        }
        let total_ms = self.gen_ms.saturating_add(current_ms);
        if total_ms == 0 {
            return None;
        }
        let completed_out = if self.out_tokens > 0 {
            self.out_tokens
        } else {
            self.out_tokens_est
        };
        let total_out = completed_out.saturating_add(current_est_out);
        if total_out == 0 {
            return None;
        }
        Some(total_out as f64 / (total_ms as f64 / 1000.0))
    }
    /// Close out one stream call: fold its generation time and output tokens
    /// into the turn totals. `tokens_out` is the real completion_tokens /
    /// output_tokens from usage (0 when the endpoint omits usage); `est_out` is
    /// the char/4 estimate retained for diagnostics/accounting only.
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
        // wall time. Only real provider usage counts as TPS: showing a char/4
        // fallback made the footer look precise while actually being a guess,
        // and it drifted badly on tool-call JSON / reasoning-heavy turns.
        let tps = if self.gen_ms > 0 && self.out_tokens > 0 {
            Some(self.out_tokens as f64 / (self.gen_ms as f64 / 1000.0))
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
        let m = vec![Message::user("hello world")];
        assert!(estimate_messages_tokens(&m) > 0);
    }

    #[test]
    fn grounded_estimate_uses_real_baseline_plus_delta() {
        // 4 messages; real prompt_tokens was recorded when the list had 2.
        let msgs = vec![
            Message::system("sys prompt here"),
            Message::user("first user message"),
            Message::assistant("assistant reply that grew the turn"),
            Message::tool("x", "tool output payload"),
        ];
        let real = 1_000u64;
        // Baseline covers msgs[0..2]; only msgs[2..4] should be char/4-estimated.
        let grounded = grounded_estimate(&msgs, Some(real), 2);
        let delta = estimate_messages_tokens(&msgs[2..]);
        assert_eq!(grounded, real + delta);
        // Must be strictly larger than re-estimating the whole list when the
        // real count exceeds the whole-list char/4 guess (which omits tool-call
        // framing, role tags, etc.) — i.e. the real baseline is authoritative.
        assert!(grounded > estimate_messages_tokens(&msgs));
    }

    #[test]
    fn grounded_estimate_falls_back_when_no_real_usage() {
        let msgs = vec![Message::user("hello world")];
        // No real baseline (first turn): behaves as a full char/4 estimate.
        assert_eq!(
            grounded_estimate(&msgs, None, 0),
            estimate_messages_tokens(&msgs)
        );
    }

    #[test]
    fn grounded_estimate_clamps_stale_length() {
        // Baseline recorded at length 10, but a rewrite shrank the list to 2.
        // Must clamp (never slice out of range) and yield just the baseline.
        let msgs = vec![Message::user("a"), Message::assistant("b")];
        let real = 500u64;
        assert_eq!(grounded_estimate(&msgs, Some(real), 10), real);
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
    fn tps_is_blank_when_provider_omits_usage() {
        let mut t = TurnTimer::new();
        // Endpoint reported no usage (out_tokens 0). Do not show the char/4
        // estimate as final TPS; it is not real model throughput.
        t.gen_ms = 4000;
        t.out_tokens = 0;
        t.out_tokens_est = 400;
        let m = t.finalize(1000, 0, 0, "test".into());
        assert!(m.tps.is_none());
    }

    #[test]
    fn live_tps_estimate_uses_current_stream_estimate() {
        let mut t = TurnTimer::new();
        t.gen_ms = 1000;
        t.out_tokens = 100;
        t.call_first_token = Some(Instant::now() - std::time::Duration::from_millis(1000));
        let live = t.live_tps_estimate(100).unwrap();
        assert!((live - 100.0).abs() < 1.0, "live tps was {live}");
    }

    #[test]
    fn tps_none_when_no_generation_time() {
        let t = TurnTimer::new();
        // Nothing streamed into gen_ms (e.g. only untimed tool-call args) → no TPS.
        let m = t.finalize(1000, 50, 0, "test".into());
        assert!(m.tps.is_none());
    }
}
