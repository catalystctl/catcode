use super::adapter::{
    normalize_http_error, BuiltProviderRequest, ProviderAdapter, ProviderError, ProviderProtocol,
    ProviderRequest,
};
use super::capabilities::ProviderCapabilities;
use super::streaming::{NormalizedStreamEvent, ToolCallDelta};
use crate::message::Message;
use serde_json::{json, Value};

pub struct CodexResponsesAdapter;

impl ProviderAdapter for CodexResponsesAdapter {
    fn id(&self) -> &'static str {
        "codex_responses"
    }
    fn protocol(&self) -> ProviderProtocol {
        ProviderProtocol::CodexResponses
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tools: true,
            parallel_tools: true,
            reasoning: true,
            vision: false,
            usage: true,
            model_discovery: true,
        }
    }
    fn build_request(&self, input: &ProviderRequest<'_>) -> Result<BuiltProviderRequest, String> {
        let values = Message::to_openai_messages(input.messages);
        let (instructions, responses_input) = responses_input(&values);
        Ok(BuiltProviderRequest {
            url: format!(
                "{}/responses",
                input.provider.base_url.trim_end_matches('/')
            ),
            body: json!({
                "model": input.model, "instructions": instructions, "input": responses_input,
                "tools": responses_tools(input.tools), "tool_choice": "auto",
                "parallel_tool_calls": true,
                "reasoning": { "effort": input.reasoning_effort, "summary": "auto" },
                "store": false, "stream": true,
                "include": ["reasoning.encrypted_content"],
            }),
            notices: Vec::new(),
        })
    }
    fn decode_stream_event(&self, value: &Value) -> Vec<NormalizedStreamEvent> {
        decode_codex_chunk(value)
    }
    fn normalize_error(&self, status: Option<u16>, body: &str) -> ProviderError {
        normalize_http_error(status, body)
    }
}

pub(crate) fn decode_codex_chunk(value: &Value) -> Vec<NormalizedStreamEvent> {
    match value.get("type").and_then(Value::as_str).unwrap_or("") {
        "response.output_text.delta" => value
            .get("delta")
            .and_then(Value::as_str)
            .map(|text| vec![NormalizedStreamEvent::TextDelta(text.to_string())])
            .unwrap_or_default(),
        "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => value
            .get("delta")
            .and_then(Value::as_str)
            .map(|text| vec![NormalizedStreamEvent::ReasoningDelta(text.to_string())])
            .unwrap_or_default(),
        "response.output_item.done" => {
            let item = value.get("item").unwrap_or(&Value::Null);
            if item.get("type").and_then(Value::as_str) != Some("function_call") {
                return Vec::new();
            }
            let delta = ToolCallDelta {
                index: value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize,
                id: item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                name: item.get("name").and_then(Value::as_str).map(str::to_string),
                arguments: item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            };
            vec![
                NormalizedStreamEvent::ToolCallStart(delta.clone()),
                NormalizedStreamEvent::ToolCallComplete { index: delta.index },
            ]
        }
        "response.completed" => value
            .get("response")
            .and_then(|response| response.get("usage"))
            .map(|usage| {
                vec![NormalizedStreamEvent::Usage {
                    input_tokens: usage.get("input_tokens").and_then(token_count),
                    output_tokens: usage.get("output_tokens").and_then(token_count),
                    cached_tokens: usage
                        .get("input_tokens_details")
                        .and_then(|details| details.get("cached_tokens"))
                        .and_then(token_count),
                }]
            })
            .unwrap_or_default(),
        "response.failed" => vec![NormalizedStreamEvent::FatalError(
            value
                .get("response")
                .and_then(|response| response.get("error"))
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Responses API failed")
                .to_string(),
        )],
        _ => Vec::new(),
    }
}

fn responses_input(messages: &[Value]) -> (String, Vec<Value>) {
    let mut instructions = Vec::new();
    let mut input = Vec::new();
    for message in messages {
        let content = content_text(message.get("content").unwrap_or(&Value::Null));
        match message.get("role").and_then(Value::as_str).unwrap_or("") {
            "system" => instructions.push(content),
            "user" => input.push(json!({"type":"message","role":"user","content":[{"type":"input_text","text":content}]})),
            "assistant" => {
                if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        input.push(json!({
                            "type":"function_call",
                            "call_id":call.get("id").and_then(Value::as_str).unwrap_or(""),
                            "name":call.get("function").and_then(|function| function.get("name")).and_then(Value::as_str).unwrap_or(""),
                            "arguments":call.get("function").and_then(|function| function.get("arguments")).and_then(Value::as_str).unwrap_or("{}"),
                        }));
                    }
                } else if !content.is_empty() {
                    input.push(json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":content}]}));
                }
            }
            "tool" => input.push(json!({
                "type":"function_call_output",
                "call_id":message.get("tool_call_id").and_then(Value::as_str).unwrap_or(""),
                "output":content,
            })),
            _ => {}
        }
    }
    (instructions.join("\n\n"), input)
}

fn responses_tools(tools: &[Value]) -> Vec<Value> {
    tools.iter().filter_map(|tool| {
        let function = tool.get("function")?;
        Some(json!({
            "type":"function", "name":function.get("name").cloned().unwrap_or(Value::Null),
            "description":function.get("description").cloned().unwrap_or(Value::Null),
            "parameters":function.get("parameters").cloned().unwrap_or_else(|| json!({"type":"object"})),
            "strict":false,
        }))
    }).collect()
}

fn content_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
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

    #[test]
    fn normalizes_codex_deltas_tools_usage_and_failure() {
        assert_eq!(
            decode_codex_chunk(&json!({"type":"response.output_text.delta","delta":"hi"})),
            vec![NormalizedStreamEvent::TextDelta("hi".into())]
        );
        assert!(matches!(
            decode_codex_chunk(&json!({"type":"response.output_item.done","output_index":3,"item":{"type":"function_call","call_id":"c","name":"read_file","arguments":"{}"}})).as_slice(),
            [NormalizedStreamEvent::ToolCallStart(delta), NormalizedStreamEvent::ToolCallComplete { index: 3 }]
                if delta.index == 3 && delta.name.as_deref() == Some("read_file")
        ));
        assert_eq!(
            decode_codex_chunk(
                &json!({"type":"response.failed","response":{"error":{"message":"bad"}}})
            ),
            vec![NormalizedStreamEvent::FatalError("bad".into())]
        );
    }
}
