use serde::Serialize;
use serde_json::Value;
use std::sync::{Arc, LazyLock};

static EVENT_SINK: LazyLock<crate::runtime::EventSink> =
    LazyLock::new(crate::runtime::EventSink::new);

pub fn install_runtime(runtime: &Arc<crate::runtime::RuntimeCoordinator>) {
    EVENT_SINK.install_runtime(runtime);
}

/// Events written to stdout. Constructed with serde_json::json! and emitted via `emit`.
#[derive(Serialize, Debug)]
pub struct Event {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(flatten)]
    pub data: serde_json::Map<String, serde_json::Value>,
}

impl Event {
    pub fn new(kind: &'static str) -> Self {
        Self {
            kind,
            data: serde_json::Map::new(),
        }
    }

    pub fn with(mut self, key: &str, value: serde_json::Value) -> Self {
        self.data.insert(key.to_string(), value);
        self
    }
}

#[cfg(test)]
type CapturedEvent = (String, serde_json::Map<String, Value>);

#[cfg(test)]
thread_local! {
    static EMIT_CAPTURE: std::cell::RefCell<Option<Vec<CapturedEvent>>> =
        const { std::cell::RefCell::new(None) };
}

/// Begin capturing emits into an in-memory buffer (unit tests). Nested calls replace.
#[cfg(test)]
pub fn begin_emit_capture() {
    EMIT_CAPTURE.with(|capture| *capture.borrow_mut() = Some(Vec::new()));
}

/// Stop capture and return `(kind, data)` pairs accumulated since [`begin_emit_capture`].
#[cfg(test)]
pub fn end_emit_capture() -> Vec<(String, serde_json::Map<String, Value>)> {
    EMIT_CAPTURE.with(|capture| capture.borrow_mut().take().unwrap_or_default())
}

/// Emit one event as a single line of JSON to stdout through the central sink.
pub fn emit(event: &Event) {
    #[cfg(test)]
    EMIT_CAPTURE.with(|capture| {
        if let Some(buffer) = capture.borrow_mut().as_mut() {
            buffer.push((event.kind.to_string(), event.data.clone()));
        }
    });
    EVENT_SINK.emit(event);
}

pub fn emit_turn_rejected(message: impl AsRef<str>) {
    emit(&Event::new("error").with("message", Value::String(message.as_ref().to_string())));
    emit(&Event::new("done"));
}

pub fn emit_aborted_done() {
    emit(&Event::new("aborted"));
    emit(&Event::new("done"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_turn_pairs_error_with_done() {
        begin_emit_capture();
        emit_turn_rejected("unknown model: nope");
        let events = end_emit_capture();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, "error");
        assert_eq!(events[0].1["message"], "unknown model: nope");
        assert_eq!(events[1].0, "done");
    }

    #[test]
    fn abort_pairs_aborted_with_done() {
        begin_emit_capture();
        emit_aborted_done();
        let events = end_emit_capture();
        assert_eq!(
            events
                .iter()
                .map(|event| event.0.as_str())
                .collect::<Vec<_>>(),
            ["aborted", "done"]
        );
    }
}
