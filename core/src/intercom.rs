// Intercom: in-process coordination channel for subagents.
//
// This is the port of pi-subagents' `pi-intercom` bridge, adapted to our
// single-process model. Because subagents run as nested agentic loops inside
// the same core process (see run_subagent in main.rs), intercom is implemented
// as in-memory mailboxes rather than file-based IPC.
//
// Two coordination tools are exposed to subagents:
//
//   contact_supervisor({ reason, message })
//     The subagent contacts the orchestrator (the parent session) that
//     delegated the task. `reason: "need_decision"` blocks until the
//     orchestrator replies (the TUI surfaces the question as an
//     `intercom_message` event and the user replies via `intercom_reply`).
//     `reason: "progress_update"` is non-blocking and returns immediately.
//     This is how subagents prompt the orchestrator for any issues.
//
//   intercom({ action, to, message, id, reply })
//     Generic peer-to-peer plumbing. Subagents can send messages to each
//     other's mailboxes, issue blocking `ask`s, poll their own mailbox, and
//     reply to pending asks. This is only available to a subagent when the
//     setup allows it: the intercom bridge mode is not "off" and the agent's
//     resolved `tools` include `intercom`.
//
// Allowed-by-setup is enforced at subagent launch time (subagent.rs decides
// whether to register these tools + inject bridge instructions); this module
// only owns the bus + the two tool bodies.

use crate::protocol::{emit, Event};
use crate::tools::Outcome;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use std::sync::Mutex;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

static ASK_SEQ: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn next_id(prefix: &str) -> String {
    let n = ASK_SEQ.fetch_add(1, Ordering::SeqCst);
    format!("{prefix}-{n:x}-{}", now_ms())
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct IntercomMessage {
    pub id: String,
    pub from: String,
    pub to: String,
    pub message: String,
    pub reason: String,
    pub ts: u64,
    /// When this message is a blocking `ask`, this is the ask id the recipient
    /// must quote in its `reply`. Empty for fire-and-forget `send`s.
    pub ask_id: String,
}

/// A pending blocking ask awaiting a reply. Stored in the bus keyed by ask id
/// until the recipient replies (or the asker is cancelled/times out).
pub struct PendingAsk {
    pub id: String,
    pub from: String,
    pub to: String,
    pub message: String,
    pub reason: String,
    pub ts: u64,
    pub reply: Mutex<Option<String>>,
    pub notify: Arc<Notify>,
}

pub struct Mailbox {
    pub target: String,
    pub messages: Mutex<VecDeque<IntercomMessage>>,
    pub notify: Notify,
}

impl Mailbox {
    fn new(target: &str) -> Self {
        Self {
            target: target.to_string(),
            messages: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
        }
    }
}

/// The shared intercom bus. Held inside State (Arc<State>) so every nested
/// subagent loop and the main stdin loop can reach it.
#[derive(Default)]
pub struct IntercomBus {
    pub mailboxes: Mutex<HashMap<String, Arc<Mailbox>>>,
    pub pending_asks: Mutex<HashMap<String, Arc<PendingAsk>>>,
    /// The orchestrator (parent session) target name. Defaults to "orchestrator".
    pub orchestrator_target: Mutex<String>,
    /// All known target names (for the `targets` action + doctor diagnostics).
    pub known_targets: Mutex<Vec<String>>,
}

impl IntercomBus {
    pub fn new() -> Self {
        let s = Self::default();
        *s.orchestrator_target.lock().unwrap() = "orchestrator".to_string();
        s.known_targets.lock().unwrap().push("orchestrator".to_string());
        s
    }

    /// The orchestrator target the parent session answers as.
    pub fn orchestrator_target(&self) -> String {
        self.orchestrator_target.lock().unwrap().clone()
    }

    /// Create (idempotently) a mailbox for a target and register it as known.
    pub fn register_target(&self, target: &str) {
        if target.is_empty() {
            return;
        }
        let mut mb = self.mailboxes.lock().unwrap();
        if !mb.contains_key(target) {
            mb.insert(target.to_string(), Arc::new(Mailbox::new(target)));
        }
        drop(mb);
        let mut kt = self.known_targets.lock().unwrap();
        if !kt.iter().any(|t| t == target) {
            kt.push(target.to_string());
        }
    }

    /// Drop a mailbox when its subagent finishes.
    pub fn unregister(&self, target: &str) {
        self.mailboxes.lock().unwrap().remove(target);
    }

    /// Known peer targets (for the `intercom({action:"targets"})` introspection
    /// action and doctor diagnostics).
    pub fn targets(&self) -> Vec<String> {
        self.known_targets.lock().unwrap().clone()
    }

    /// Post a fire-and-forget message into a target's mailbox. Returns Err if
    /// the target is unknown (no registered mailbox and not the orchestrator).
    pub fn post(&self, msg: IntercomMessage) -> Result<(), String> {
        let target = msg.to.clone();
        // The orchestrator always exists as a conceptual target even if no
        // mailbox was explicitly created for it (it answers via the TUI).
        if target != self.orchestrator_target() {
            let mb = self.mailboxes.lock().unwrap();
            if !mb.contains_key(&target) {
                return Err(format!(
                    "unknown intercom target '{target}'; use action:\"targets\" to list known peers"
                ));
            }
        }
        // If the recipient has a mailbox, push there. The orchestrator has no
        // mailbox (it answers via events), so its messages are surfaced by the
        // caller through `emit_intercom_message` instead.
        let mailbox = self.mailboxes.lock().unwrap().get(&target).cloned();
        if let Some(mb) = mailbox {
            mb.messages.lock().unwrap().push_back(msg.clone());
            mb.notify.notify_one();
        }
        Ok(())
    }

    /// Read (and remove) the oldest message from a target's mailbox.
    pub fn receive(&self, target: &str) -> Option<IntercomMessage> {
        let mb = {
            let guard = self.mailboxes.lock().unwrap();
            guard.get(target).cloned()
        }?;
        let msg = mb.messages.lock().unwrap().pop_front();
        msg
    }

    /// Register a blocking ask and return its handle. The caller awaits the
    /// handle's notify; the recipient resolves it via `resolve_ask`.
    pub fn create_ask(&self, ask: PendingAsk) -> Arc<PendingAsk> {
        let arc = Arc::new(ask);
        {
            let mut pa = self.pending_asks.lock().unwrap();
            pa.insert(arc.id.clone(), arc.clone());
        }
        // Surface it: if addressed to the orchestrator, emit an event so the
        // TUI/user can reply; otherwise drop it into the recipient mailbox so a
        // peer subagent can `receive` it and `reply`.
        if arc.to == self.orchestrator_target() {
            emit_intercom_message(&arc);
        } else {
            let msg = IntercomMessage {
                id: arc.id.clone(),
                from: arc.from.clone(),
                to: arc.to.clone(),
                message: arc.message.clone(),
                reason: arc.reason.clone(),
                ts: arc.ts,
                ask_id: arc.id.clone(),
            };
            let _ = self.post(msg);
        }
        arc
    }

    /// Resolve a pending ask with a reply. Returns true if the ask existed.
    pub fn resolve_ask(&self, id: &str, reply: &str) -> bool {
        let ask = self.pending_asks.lock().unwrap().remove(id);
        if let Some(ask) = ask {
            *ask.reply.lock().unwrap() = Some(reply.to_string());
            ask.notify.notify_one();
            true
        } else {
            false
        }
    }

    /// Take a pending ask (remove it) without resolving — used on cancel/abort
    /// so the awaiting task can wake and return.
    pub fn cancel_ask(&self, id: &str) {
        let ask = self.pending_asks.lock().unwrap().remove(id);
        if let Some(ask) = ask {
            *ask.reply.lock().unwrap() = Some("[interrupted]".to_string());
            ask.notify.notify_one();
        }
    }

    /// Snapshot of pending ask ids (doctor diagnostics).
    pub fn pending_count(&self) -> usize {
        self.pending_asks.lock().unwrap().len()
    }
}

/// Emit the `intercom_message` event the TUI surfaces as a prompt for the
/// orchestrator to reply. Mirrors the approval_request flow.
fn emit_intercom_message(ask: &PendingAsk) {
    emit(
        &Event::new("intercom_message")
            .with("id", json!(ask.id))
            .with("from", json!(ask.from))
            .with("to", json!(ask.to))
            .with("reason", json!(ask.reason))
            .with("message", json!(ask.message)),
    );
}

/// Execute the `contact_supervisor` tool from within a subagent loop.
///
/// `from` is the calling subagent's intercom target; `orchestrator` is the
/// bus's orchestrator target. `reason: "need_decision"` blocks for a reply
/// (until the orchestrator answers via `intercom_reply`, or the turn is
/// cancelled); `reason: "progress_update"` returns immediately.
pub async fn execute_contact_supervisor(
    args: &Value,
    bus: &IntercomBus,
    from: &str,
    cancel: &CancellationToken,
) -> Outcome {
    let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("need_decision");
    let message = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
    if message.is_empty() {
        return Outcome::err("contact_supervisor requires a 'message'");
    }
    let orchestrator = bus.orchestrator_target();
    let ask = PendingAsk {
        id: next_id("ask"),
        from: from.to_string(),
        to: orchestrator.clone(),
        message: message.to_string(),
        reason: reason.to_string(),
        ts: now_ms(),
        reply: Mutex::new(None),
        notify: Arc::new(Notify::new()),
    };

    // progress_update is non-blocking: surface it but return immediately.
    if reason == "progress_update" {
        emit(
            &Event::new("intercom_message")
                .with("id", json!(ask.id))
                .with("from", json!(from))
                .with("to", json!(orchestrator))
                .with("reason", json!("progress_update"))
                .with("message", json!(message)),
        );
        return Outcome::ok("progress update sent to supervisor");
    }

    let handle = bus.create_ask(ask);
    // Block for the reply, or bail out on cancel.
    let result = tokio::select! {
        _ = handle.notify.notified() => {
            let reply = handle.reply.lock().unwrap().clone();
            reply.unwrap_or_else(|| "[no reply]".to_string())
        }
        _ = cancel.cancelled() => {
            bus.cancel_ask(&handle.id);
            "[interrupted]".to_string()
        }
    };
    Outcome::ok(result)
}

/// Execute the `intercom` tool: peer-to-peer plumbing between subagents.
pub async fn execute_intercom(
    args: &Value,
    bus: &IntercomBus,
    from: &str,
    cancel: &CancellationToken,
) -> Outcome {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("send");
    match action {
        "targets" => {
            let t = bus.targets();
            return Outcome::ok(json!(t).to_string());
        }
        "receive" | "poll" => {
            return match bus.receive(from) {
                Some(m) => Outcome::ok(json!(m).to_string()),
                None => Outcome::ok("[]"), // no pending messages
            };
        }
        "send" => {
            let to = args.get("to").and_then(|v| v.as_str()).unwrap_or("");
            let message = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
            if to.is_empty() || message.is_empty() {
                return Outcome::err("intercom send requires 'to' and 'message'");
            }
            let msg = IntercomMessage {
                id: next_id("msg"),
                from: from.to_string(),
                to: to.to_string(),
                message: message.to_string(),
                reason: args.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                ts: now_ms(),
                ask_id: String::new(),
            };
            // The orchestrator answers via events, not a mailbox.
            if to == bus.orchestrator_target() {
                emit(
                    &Event::new("intercom_message")
                        .with("id", json!(msg.id))
                        .with("from", json!(from))
                        .with("to", json!(to))
                        .with("reason", json!(msg.reason))
                        .with("message", json!(message)),
                );
                return Outcome::ok(format!("message sent to {to}"));
            }
            match bus.post(msg) {
                Ok(()) => Outcome::ok(format!("message sent to {to}")),
                Err(e) => Outcome::err(e),
            }
        }
        "ask" => {
            // Blocking ask to a peer subagent (or the orchestrator).
            let to = args.get("to").and_then(|v| v.as_str()).unwrap_or("");
            let message = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("need_decision");
            if to.is_empty() || message.is_empty() {
                return Outcome::err("intercom ask requires 'to' and 'message'");
            }
            let ask = PendingAsk {
                id: next_id("ask"),
                from: from.to_string(),
                to: to.to_string(),
                message: message.to_string(),
                reason: reason.to_string(),
                ts: now_ms(),
                reply: Mutex::new(None),
                notify: Arc::new(Notify::new()),
            };
            let handle = bus.create_ask(ask);
            let result = tokio::select! {
                _ = handle.notify.notified() => {
                    handle.reply.lock().unwrap().clone().unwrap_or_else(|| "[no reply]".to_string())
                }
                _ = cancel.cancelled() => {
                    bus.cancel_ask(&handle.id);
                    "[interrupted]".to_string()
                }
            };
            Outcome::ok(result)
        }
        "reply" => {
            // Reply to a pending ask addressed to `from` (the caller).
            let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let reply = args.get("reply").and_then(|v| v.as_str()).unwrap_or("");
            if id.is_empty() {
                return Outcome::err("intercom reply requires 'id' (the ask id) and 'reply'");
            }
            if bus.resolve_ask(id, reply) {
                Outcome::ok(format!("replied to ask {id}"))
            } else {
                Outcome::err(format!("no pending ask with id '{id}' (it may have timed out or been answered)"))
            }
        }
        other => Outcome::err(format!("unknown intercom action '{other}'; use send|ask|receive|poll|reply|targets")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_send_to_peer() {
        let bus = IntercomBus::new();
        bus.register_target("subagent-worker-1");
        bus.register_target("subagent-reviewer-1");
        let msg = IntercomMessage {
            id: "m1".into(),
            from: "subagent-worker-1".into(),
            to: "subagent-reviewer-1".into(),
            message: "hi".into(),
            reason: "".into(),
            ts: 1,
            ask_id: "".into(),
        };
        assert!(bus.post(msg).is_ok());
        let got = bus.receive("subagent-reviewer-1").unwrap();
        assert_eq!(got.message, "hi");
        assert!(bus.receive("subagent-reviewer-1").is_none());
    }

    #[test]
    fn send_to_unknown_target_errors() {
        let bus = IntercomBus::new();
        let msg = IntercomMessage {
            id: "m1".into(),
            from: "a".into(),
            to: "ghost".into(),
            message: "hi".into(),
            reason: "".into(),
            ts: 1,
            ask_id: "".into(),
        };
        assert!(bus.post(msg).is_err());
    }

    #[test]
    fn orchestrator_is_known_default() {
        let bus = IntercomBus::new();
        assert_eq!(bus.orchestrator_target(), "orchestrator");
        assert!(bus.targets().contains(&"orchestrator".to_string()));
    }

    #[test]
    fn resolve_ask_returns_reply() {
        let bus = IntercomBus::new();
        let ask = PendingAsk {
            id: "a1".into(),
            from: "child".into(),
            to: "orchestrator".into(),
            message: "decide?".into(),
            reason: "need_decision".into(),
            ts: 1,
            reply: Mutex::new(None),
            notify: Arc::new(Notify::new()),
        };
        let handle = bus.create_ask(ask);
        assert!(bus.resolve_ask("a1", "do it"));
        assert_eq!(handle.reply.lock().unwrap().clone().unwrap(), "do it");
        // second resolve fails (already removed)
        assert!(!bus.resolve_ask("a1", "again"));
    }
}
