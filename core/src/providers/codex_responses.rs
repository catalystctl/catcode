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
        let mut notices = Vec::new();
        let effort = normalize_codex_reasoning_effort(
            input.reasoning_effort,
            input.thinking_levels,
            &mut notices,
        );

        // Build the body field-by-field so optional wire fields (max_output_tokens)
        // can be omitted entirely rather than sent as 0.
        let mut body = json!({
            "model": input.model,
            "instructions": instructions,
            "input": responses_input,
            "tools": responses_tools(input.tools),
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            // Never persist Responses API turns server-side — the harness owns
            // session history. store:true would leak prompts/tool I/O to OpenAI.
            "store": false,
            "stream": true,
            "include": ["reasoning.encrypted_content"],
            "reasoning": { "effort": effort, "summary": "auto" },
        });
        // Responses API uses max_output_tokens (not max_tokens). Some gateways
        // reject or mis-handle 0 as "generate nothing". Only send a real positive
        // budget from model discovery; omit otherwise so the server default applies
        // (no artificial harness cap).
        if input.max_tokens > 0 {
            body["max_output_tokens"] = json!(input.max_tokens);
        }

        Ok(BuiltProviderRequest {
            url: format!(
                "{}/responses",
                input.provider.base_url.trim_end_matches('/')
            ),
            body,
            notices,
        })
    }
    fn decode_stream_event(&self, value: &Value) -> Vec<NormalizedStreamEvent> {
        decode_codex_chunk(value)
    }
    fn normalize_error(&self, status: Option<u16>, body: &str) -> ProviderError {
        normalize_http_error(status, body)
    }
}

/// Clamp / default the Responses API `reasoning.effort` value.
/// Empty effort becomes `"medium"` (safe default); when the model advertises
/// levels, unsupported values are resolved via [`crate::provider::resolve_effort`].
fn normalize_codex_reasoning_effort(
    requested: &str,
    levels: &[String],
    notices: &mut Vec<String>,
) -> String {
    let requested = requested.trim();
    let requested = if requested.is_empty() {
        "medium"
    } else {
        requested
    };
    let resolved = crate::provider::resolve_effort(requested, levels);
    if !resolved.eq_ignore_ascii_case(requested) {
        notices.push(format!(
            "reasoning effort '{}' not supported; using '{}'",
            requested, resolved
        ));
    }
    resolved
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
        "response.completed" => {
            let mut events = Vec::new();
            if let Some(usage) = value
                .get("response")
                .and_then(|response| response.get("usage"))
            {
                events.push(NormalizedStreamEvent::Usage {
                    input_tokens: usage.get("input_tokens").and_then(token_count),
                    output_tokens: usage.get("output_tokens").and_then(token_count),
                    cached_tokens: usage
                        .get("input_tokens_details")
                        .and_then(|details| details.get("cached_tokens"))
                        .and_then(token_count),
                });
            }
            events
        }
        // Hit the output token budget (or other incomplete status). Surface as
        // finish_reason=length so the agent loop can react instead of treating
        // a truncated reply as a clean "stop".
        "response.incomplete" => {
            let mut events = Vec::new();
            if let Some(usage) = value
                .get("response")
                .and_then(|response| response.get("usage"))
            {
                events.push(NormalizedStreamEvent::Usage {
                    input_tokens: usage.get("input_tokens").and_then(token_count),
                    output_tokens: usage.get("output_tokens").and_then(token_count),
                    cached_tokens: usage
                        .get("input_tokens_details")
                        .and_then(|details| details.get("cached_tokens"))
                        .and_then(token_count),
                });
            }
            events.push(NormalizedStreamEvent::FinishReason("length".into()));
            events
        }
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
            "system" => {
                if !content.is_empty() {
                    instructions.push(content);
                }
            }
            "user" => input.push(json!({"type":"message","role":"user","content":[{"type":"input_text","text":content}]})),
            "assistant" => {
                // Preserve assistant prose even when tool_calls are present —
                // dropping content used to erase the model's narration from the
                // next-turn Responses `input` history.
                if !content.is_empty() {
                    input.push(json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":content}]}));
                }
                if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        input.push(json!({
                            "type":"function_call",
                            "call_id":call.get("id").and_then(Value::as_str).unwrap_or(""),
                            "name":call.get("function").and_then(|function| function.get("name")).and_then(Value::as_str).unwrap_or(""),
                            "arguments":call.get("function").and_then(|function| function.get("arguments")).and_then(Value::as_str).unwrap_or("{}"),
                        }));
                    }
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
    tools
        .iter()
        .filter_map(|tool| {
            let function = tool.get("function")?;
            // Skip schema-less / nameless tool entries — Responses rejects
            // function tools without a non-empty name.
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())?;
            Some(json!({
                "type": "function",
                "name": name,
                "description": function.get("description").cloned().unwrap_or(Value::Null),
                "parameters": function
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({"type":"object","properties":{}})),
                "strict": false,
            }))
        })
        .collect()
}

fn content_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                // OpenAI chat content parts use `text`; tolerate either.
                part.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| part.as_str())
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn token_count(value: &Value) -> Option<u64> {
    if let Some(n) = value.as_u64() {
        return Some(n);
    }
    // Reject negatives before the `as u64` cast (which would wrap to u64::MAX
    // and corrupt session token accounting).
    if let Some(n) = value.as_i64() {
        return if n >= 0 { Some(n as u64) } else { None };
    }
    if let Some(n) = value.as_f64() {
        if n.is_finite() && n >= 0.0 && n <= u64::MAX as f64 {
            return Some(n as u64);
        }
        return None;
    }
    value.as_str()?.trim().parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::super::adapter::ProviderRequest;
    use super::*;
    use crate::config::{ProviderKind, ResolvedProvider};

    fn fixture_provider() -> ResolvedProvider {
        ResolvedProvider {
            name: "codex".into(),
            kind: ProviderKind::OpenAI,
            base_url: "https://chatgpt.com/backend-api/codex".into(),
            api_key: None,
            headers: Vec::new(),
            oauth: true,
            context_window: None,
            models_override: Vec::new(),
            models_endpoint: None,
        }
    }

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

    #[test]
    fn incomplete_maps_to_finish_reason_length() {
        let events = decode_codex_chunk(&json!({
            "type": "response.incomplete",
            "response": {
                "usage": {"input_tokens": 10, "output_tokens": 50}
            }
        }));
        assert!(events.iter().any(|e| matches!(
            e,
            NormalizedStreamEvent::FinishReason(r) if r == "length"
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            NormalizedStreamEvent::Usage {
                input_tokens: Some(10),
                output_tokens: Some(50),
                ..
            }
        )));
    }

    #[test]
    fn negative_token_counts_are_dropped() {
        let events = decode_codex_chunk(&json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": -1.0,
                    "output_tokens": 7,
                    "input_tokens_details": {"cached_tokens": -3}
                }
            }
        }));
        assert_eq!(
            events,
            vec![NormalizedStreamEvent::Usage {
                input_tokens: None,
                output_tokens: Some(7),
                cached_tokens: None,
            }]
        );
    }

    #[test]
    fn max_output_tokens_positive_is_sent_zero_is_omitted() {
        let provider = fixture_provider();
        let messages: Vec<Message> =
            serde_json::from_value(json!([{"role":"user","content":"hi"}])).unwrap();

        let with_budget = CodexResponsesAdapter
            .build_request(&ProviderRequest {
                provider: &provider,
                model: "gpt-5.5",
                messages: &messages,
                tools: &[],
                reasoning_effort: "medium",
                thinking_levels: &[],
                max_tokens: 4096,
            })
            .unwrap();
        assert_eq!(with_budget.body["max_output_tokens"], 4096);
        assert!(with_budget.body.get("max_tokens").is_none());
        assert_eq!(with_budget.body["store"], false);

        let zero = CodexResponsesAdapter
            .build_request(&ProviderRequest {
                provider: &provider,
                model: "gpt-5.5",
                messages: &messages,
                tools: &[],
                reasoning_effort: "medium",
                thinking_levels: &[],
                max_tokens: 0,
            })
            .unwrap();
        assert!(
            zero.body.get("max_output_tokens").is_none(),
            "max_output_tokens:0 must be omitted, got {:?}",
            zero.body.get("max_output_tokens")
        );
        assert_eq!(zero.body["store"], false);
    }

    #[test]
    fn empty_reasoning_effort_defaults_and_levels_clamp() {
        let provider = fixture_provider();
        let messages: Vec<Message> =
            serde_json::from_value(json!([{"role":"user","content":"hi"}])).unwrap();
        let levels = vec!["low".into(), "high".into()];

        let empty = CodexResponsesAdapter
            .build_request(&ProviderRequest {
                provider: &provider,
                model: "gpt-5.5",
                messages: &messages,
                tools: &[],
                reasoning_effort: "",
                thinking_levels: &levels,
                max_tokens: 0,
            })
            .unwrap();
        // empty → "medium" → clamped to preferred supported ("high")
        assert_eq!(empty.body["reasoning"]["effort"], "high");
        assert!(!empty.notices.is_empty());

        let ok = CodexResponsesAdapter
            .build_request(&ProviderRequest {
                provider: &provider,
                model: "gpt-5.5",
                messages: &messages,
                tools: &[],
                reasoning_effort: "low",
                thinking_levels: &levels,
                max_tokens: 0,
            })
            .unwrap();
        assert_eq!(ok.body["reasoning"]["effort"], "low");
    }

    #[test]
    fn system_prompt_and_assistant_content_with_tools_preserved() {
        let (instructions, input) = responses_input(&[
            json!({"role":"system","content":"Be helpful."}),
            json!({"role":"user","content":"run it"}),
            json!({
                "role":"assistant",
                "content":"Calling the tool.",
                "tool_calls":[{
                    "id":"c1",
                    "type":"function",
                    "function":{"name":"bash","arguments":"{\"command\":\"ls\"}"}
                }]
            }),
            json!({"role":"tool","tool_call_id":"c1","content":"ok"}),
        ]);
        assert_eq!(instructions, "Be helpful.");
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        // Assistant prose kept ahead of the function_call item.
        assert_eq!(input[1]["type"], "message");
        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[1]["content"][0]["text"], "Calling the tool.");
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[2]["call_id"], "c1");
        assert_eq!(input[3]["type"], "function_call_output");
    }

    #[test]
    fn tools_without_name_are_dropped() {
        let tools = responses_tools(&[
            json!({"type":"function","function":{"name":"read_file","parameters":{"type":"object"}}}),
            json!({"type":"function","function":{"description":"nameless"}}),
            json!({"type":"function","function":{"name":""}}),
        ]);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "read_file");
        assert_eq!(tools[0]["strict"], false);
    }
}
