use super::adapter::{
    normalize_http_error, BuiltProviderRequest, ProviderAdapter, ProviderError, ProviderProtocol,
    ProviderRequest,
};
use super::capabilities::ProviderCapabilities;
use super::streaming::{NormalizedStreamEvent, ToolCallDelta};
use serde_json::Value;

pub struct AnthropicCompatibleAdapter;

impl ProviderAdapter for AnthropicCompatibleAdapter {
    fn id(&self) -> &'static str {
        "anthropic_compatible"
    }

    fn protocol(&self) -> ProviderProtocol {
        ProviderProtocol::AnthropicMessages
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tools: true,
            parallel_tools: true,
            reasoning: true,
            vision: true,
            usage: true,
            model_discovery: true,
        }
    }

    fn build_request(&self, input: &ProviderRequest<'_>) -> Result<BuiltProviderRequest, String> {
        let max_tokens = if input.max_tokens == 0 {
            8192
        } else {
            input.max_tokens
        };
        let mut body = crate::message::build_anthropic_request(
            input.messages,
            input.tools,
            input.reasoning_effort,
            input.thinking_levels,
            max_tokens,
        );
        body["stream"] = Value::Bool(true);
        body["model"] = Value::String(input.model.to_string());
        Ok(BuiltProviderRequest {
            url: format!("{}/messages", input.provider.base_url.trim_end_matches('/')),
            body,
            notices: Vec::new(),
        })
    }

    fn decode_stream_event(&self, value: &Value) -> Vec<NormalizedStreamEvent> {
        decode_anthropic_chunk(value)
    }

    fn normalize_error(&self, status: Option<u16>, body: &str) -> ProviderError {
        normalize_http_error(status, body)
    }
}

pub(crate) fn decode_anthropic_chunk(value: &Value) -> Vec<NormalizedStreamEvent> {
    let mut events = Vec::new();
    match value.get("type").and_then(Value::as_str).unwrap_or("") {
        "message_start" => {
            let usage = value
                .get("message")
                .and_then(|message| message.get("usage"));
            if let Some(usage) = usage {
                events.push(NormalizedStreamEvent::Usage {
                    input_tokens: usage.get("input_tokens").and_then(token_count),
                    output_tokens: usage.get("output_tokens").and_then(token_count),
                    cached_tokens: usage.get("cache_read_input_tokens").and_then(token_count),
                });
            }
        }
        "content_block_start" => {
            let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let block = value.get("content_block").unwrap_or(&Value::Null);
            if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                events.push(NormalizedStreamEvent::ToolCallStart(ToolCallDelta {
                    index,
                    id: block.get("id").and_then(Value::as_str).map(str::to_string),
                    name: block
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    arguments: None,
                }));
            }
        }
        "content_block_delta" => {
            let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let delta = value.get("delta").unwrap_or(&Value::Null);
            match delta.get("type").and_then(Value::as_str).unwrap_or("") {
                "text_delta" => {
                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                        events.push(NormalizedStreamEvent::TextDelta(text.to_string()));
                    }
                }
                "thinking_delta" => {
                    if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                        events.push(NormalizedStreamEvent::ReasoningDelta(text.to_string()));
                    }
                }
                "input_json_delta" => {
                    events.push(NormalizedStreamEvent::ToolCallDelta(ToolCallDelta {
                        index,
                        id: None,
                        name: None,
                        arguments: delta
                            .get("partial_json")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    }));
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            events.push(NormalizedStreamEvent::ToolCallComplete { index });
        }
        "message_delta" => {
            if let Some(reason) = value
                .get("delta")
                .and_then(|delta| delta.get("stop_reason"))
                .and_then(Value::as_str)
            {
                events.push(NormalizedStreamEvent::FinishReason(
                    match reason {
                        "end_turn" | "stop_sequence" => "stop",
                        "tool_use" => "tool_calls",
                        "max_tokens" => "length",
                        other => other,
                    }
                    .to_string(),
                ));
            }
            if let Some(usage) = value.get("usage") {
                events.push(NormalizedStreamEvent::Usage {
                    input_tokens: usage.get("input_tokens").and_then(token_count),
                    output_tokens: usage.get("output_tokens").and_then(token_count),
                    cached_tokens: usage.get("cache_read_input_tokens").and_then(token_count),
                });
            }
        }
        "error" => {
            let error = value.get("error").unwrap_or(value);
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("anthropic stream error")
                .to_string();
            let retryable = error
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| matches!(kind, "overloaded_error" | "rate_limit_error"));
            events.push(if retryable {
                NormalizedStreamEvent::RetryableError(message)
            } else {
                NormalizedStreamEvent::FatalError(message)
            });
        }
        _ => {}
    }
    events
}

fn token_count(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_f64().map(|number| number as u64))
        .or_else(|| value.as_str()?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_anthropic_text_tools_usage_finish_and_errors() {
        assert_eq!(
            decode_anthropic_chunk(
                &json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}})
            ),
            vec![NormalizedStreamEvent::TextDelta("hi".into())]
        );
        assert!(matches!(
            decode_anthropic_chunk(&json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"call","name":"read_file","input":{}}})).as_slice(),
            [NormalizedStreamEvent::ToolCallStart(delta)] if delta.index == 2 && delta.id.as_deref() == Some("call")
        ));
        assert_eq!(
            decode_anthropic_chunk(
                &json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":4}})
            ),
            vec![
                NormalizedStreamEvent::FinishReason("tool_calls".into()),
                NormalizedStreamEvent::Usage {
                    input_tokens: None,
                    output_tokens: Some(4),
                    cached_tokens: None
                }
            ]
        );
        assert_eq!(
            decode_anthropic_chunk(
                &json!({"type":"error","error":{"type":"overloaded_error","message":"busy"}})
            ),
            vec![NormalizedStreamEvent::RetryableError("busy".into())]
        );
    }
}
