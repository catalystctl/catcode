use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolCallDelta {
    pub index: usize,
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NormalizedStreamEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallStart(ToolCallDelta),
    ToolCallDelta(ToolCallDelta),
    ToolCallComplete {
        index: usize,
    },
    Usage {
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cached_tokens: Option<u64>,
    },
    FinishReason(String),
    RetryableError(String),
    FatalError(String),
}

fn token_count(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_f64().map(|number| number as u64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

/// Normalize one parsed OpenAI-compatible SSE data object. Transport framing,
/// retry decisions, accumulation, and user-visible emission remain outside
/// this pure decoder.
pub(crate) fn decode_openai_chunk(object: &Value) -> Vec<NormalizedStreamEvent> {
    let mut events = Vec::new();
    if let Some(error) = object.get("error") {
        let detail = error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string());
        let retryable = error
            .get("code")
            .and_then(Value::as_str)
            .is_some_and(|code| matches!(code, "rate_limit" | "server_error" | "overloaded"));
        events.push(if retryable {
            NormalizedStreamEvent::RetryableError(detail)
        } else {
            NormalizedStreamEvent::FatalError(detail)
        });
        return events;
    }
    if let Some(usage) = object.get("usage") {
        events.push(NormalizedStreamEvent::Usage {
            input_tokens: usage.get("prompt_tokens").and_then(token_count),
            output_tokens: usage.get("completion_tokens").and_then(token_count),
            cached_tokens: usage
                .get("prompt_tokens_details")
                .and_then(|details| details.get("cached_tokens"))
                .and_then(token_count),
        });
    }
    let Some(choice) = object.get("choices").and_then(|choices| choices.get(0)) else {
        return events;
    };
    if let Some(delta) = choice.get("delta") {
        if let Some(text) = delta
            .get("content")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            events.push(NormalizedStreamEvent::TextDelta(text.to_string()));
        }
        if let Some(reasoning) = delta
            .get("reasoning_content")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            events.push(NormalizedStreamEvent::ReasoningDelta(reasoning.to_string()));
        }
        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            events.extend(tool_calls.iter().map(|tool_call| {
                let function = tool_call.get("function");
                NormalizedStreamEvent::ToolCallDelta(ToolCallDelta {
                    index: tool_call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize,
                    id: tool_call
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    name: function
                        .and_then(|value| value.get("name"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    arguments: function
                        .and_then(|value| value.get("arguments"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            }));
        }
    }
    if let Some(reason) = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .filter(|reason| !reason.is_empty())
    {
        events.push(NormalizedStreamEvent::FinishReason(reason.to_string()));
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let text = match name {
            "text" => include_str!("../../tests/fixtures/providers/openai_text.json"),
            "tools" => include_str!("../../tests/fixtures/providers/openai_tools.json"),
            "error" => include_str!("../../tests/fixtures/providers/openai_error.json"),
            _ => unreachable!(),
        };
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn normalizes_text_reasoning_usage_and_finish() {
        assert_eq!(
            decode_openai_chunk(&fixture("text")),
            vec![
                NormalizedStreamEvent::Usage {
                    input_tokens: Some(10),
                    output_tokens: Some(2),
                    cached_tokens: Some(4),
                },
                NormalizedStreamEvent::TextDelta("hello".into()),
                NormalizedStreamEvent::ReasoningDelta("think".into()),
                NormalizedStreamEvent::FinishReason("stop".into()),
            ]
        );
    }

    #[test]
    fn normalizes_multiple_fragmented_tool_calls() {
        let events = decode_openai_chunk(&fixture("tools"));
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            NormalizedStreamEvent::ToolCallDelta(delta)
                if delta.index == 0 && delta.name.as_deref() == Some("read_file")
        ));
        assert!(matches!(
            &events[1],
            NormalizedStreamEvent::ToolCallDelta(delta)
                if delta.index == 1 && delta.arguments.as_deref() == Some("{\"path\":")
        ));
    }

    #[test]
    fn normalizes_provider_error_without_panicking() {
        assert_eq!(
            decode_openai_chunk(&fixture("error")),
            vec![NormalizedStreamEvent::RetryableError("busy".into())]
        );
    }
}
