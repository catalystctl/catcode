use super::adapter::{
    normalize_http_error, BuiltProviderRequest, ProviderAdapter, ProviderError, ProviderProtocol,
    ProviderRequest,
};
use super::capabilities::ProviderCapabilities;
use super::streaming::{NormalizedStreamEvent, ToolCallDelta};
use crate::message::{Content, ContentPart, Message};
use serde_json::{json, Value};
use std::collections::HashMap;

pub struct GoogleCodeAssistAdapter;

impl ProviderAdapter for GoogleCodeAssistAdapter {
    fn id(&self) -> &'static str {
        "google_code_assist"
    }
    fn protocol(&self) -> ProviderProtocol {
        ProviderProtocol::GoogleCodeAssist
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
        let project = input
            .provider
            .headers
            .iter()
            .find(|(key, _)| {
                matches!(
                    key.to_ascii_lowercase().as_str(),
                    "x-goog-user-project" | "cloudaicompanion-project" | "x-code-assist-project"
                )
            })
            .map(|(_, value)| value.clone())
            .or_else(|| std::env::var("CODE_ASSIST_PROJECT").ok())
            .or_else(|| std::env::var("GOOGLE_CLOUD_PROJECT").ok())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "rising-fact-p41fc".into());
        let model = resolve_model_id(input.model, input.reasoning_effort);
        let (contents, system_instruction) = messages_to_contents(input.messages);
        let mut body = json!({
            "model": model, "project": project, "userAgent": "antigravity",
            "request": { "contents": contents, "generationConfig": { "maxOutputTokens": input.max_tokens } }
        });
        if let Some(system) = system_instruction {
            body["request"]["systemInstruction"] = system;
        }
        let tools = tools_to_genai(input.tools);
        if !tools.is_empty() {
            body["request"]["tools"] = json!(tools);
        }
        apply_thinking(&mut body, &model, input.reasoning_effort);
        Ok(BuiltProviderRequest {
            url: format!(
                "{}:streamGenerateContent?alt=sse",
                input.provider.base_url.trim_end_matches('/')
            ),
            body,
            notices: Vec::new(),
        })
    }
    fn decode_stream_event(&self, value: &Value) -> Vec<NormalizedStreamEvent> {
        decode_google_chunk(value)
    }
    fn normalize_error(&self, status: Option<u16>, body: &str) -> ProviderError {
        normalize_http_error(status, body)
    }
}

pub(crate) fn decode_google_chunk(value: &Value) -> Vec<NormalizedStreamEvent> {
    let response = value.get("response").unwrap_or(value);
    let mut events = Vec::new();
    if let Some(usage) = response.get("usageMetadata") {
        events.push(NormalizedStreamEvent::Usage {
            input_tokens: usage.get("promptTokenCount").and_then(token_count),
            output_tokens: usage.get("candidatesTokenCount").and_then(token_count),
            cached_tokens: usage.get("cachedContentTokenCount").and_then(token_count),
        });
    }
    let Some(candidate) = response
        .get("candidates")
        .and_then(|candidates| candidates.get(0))
    else {
        return events;
    };
    if let Some(parts) = candidate
        .get("content")
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)
    {
        for (index, part) in parts.iter().enumerate() {
            if let Some(text) = part
                .get("text")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                events.push(
                    if part
                        .get("thought")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        NormalizedStreamEvent::ReasoningDelta(text.to_string())
                    } else {
                        NormalizedStreamEvent::TextDelta(text.to_string())
                    },
                );
            }
            if let Some(call) = part.get("functionCall") {
                events.push(NormalizedStreamEvent::ToolCallStart(ToolCallDelta {
                    index,
                    id: None,
                    name: call.get("name").and_then(Value::as_str).map(str::to_string),
                    arguments: Some(
                        call.get("args")
                            .cloned()
                            .unwrap_or_else(|| json!({}))
                            .to_string(),
                    ),
                }));
                events.push(NormalizedStreamEvent::ToolCallComplete { index });
            }
        }
    }
    if let Some(reason) = candidate
        .get("finishReason")
        .and_then(Value::as_str)
        .filter(|reason| !reason.is_empty() && *reason != "FINISH_REASON_UNSPECIFIED")
    {
        events.push(NormalizedStreamEvent::FinishReason(reason.to_string()));
    }
    events
}

fn resolve_model_id(model: &str, reasoning_effort: &str) -> String {
    let id = model.strip_prefix("models/").unwrap_or(model);
    let mut id = id.strip_prefix("antigravity-").unwrap_or(id).to_string();
    let lower = id.to_ascii_lowercase();
    let pro = (lower.starts_with("gemini-3") || lower.starts_with("gemini-3.1"))
        && lower.contains("pro")
        && !lower.contains("flash");
    if pro && !lower.ends_with("-low") && !lower.ends_with("-high") {
        let tier = if matches!(
            reasoning_effort.to_ascii_lowercase().as_str(),
            "low" | "minimal" | "none" | ""
        ) {
            "low"
        } else {
            "high"
        };
        id = format!("{id}-{tier}");
    }
    id
}

fn apply_thinking(request: &mut Value, model: &str, reasoning_effort: &str) {
    let lower = model.to_ascii_lowercase();
    let effort = reasoning_effort.to_ascii_lowercase();
    let off = matches!(effort.as_str(), "" | "none" | "off");
    if lower.contains("gemini-3") {
        let mut level = if off {
            if lower.contains("flash") {
                "minimal"
            } else {
                "low"
            }
        } else {
            match effort.as_str() {
                "minimal" => "minimal",
                "low" => "low",
                "medium" => "medium",
                "high" | "max" => "high",
                _ if lower.contains("flash") => "medium",
                _ => "high",
            }
        };
        if !lower.contains("flash") && level != "high" {
            level = "low";
        }
        request["request"]["generationConfig"]["thinkingConfig"] =
            json!({"thinkingLevel":level,"includeThoughts":true});
    } else if off {
        request["request"]["generationConfig"]["thinkingConfig"] = json!({"thinkingBudget":0});
    } else {
        let budget = match effort.as_str() {
            "low" | "minimal" => 8192,
            "high" | "max" => 32768,
            _ => 16384,
        };
        request["request"]["generationConfig"]["thinkingConfig"] =
            json!({"thinkingBudget":budget,"includeThoughts":true});
    }
}

fn messages_to_contents(messages: &[Message]) -> (Vec<Value>, Option<Value>) {
    let mut contents = Vec::new();
    let mut system = Vec::new();
    let mut call_names = HashMap::<String, String>::new();
    for message in messages {
        match message {
            Message::System { content, .. } => {
                let text = content_text(content);
                if !text.is_empty() {
                    system.push(json!({"text":text}));
                }
            }
            Message::User { content, .. } => {
                contents.push(json!({"role":"user","parts":[{"text":content_text(content)}]}))
            }
            Message::Assistant {
                content,
                tool_calls,
                ..
            } => {
                call_names.clear();
                let mut parts = Vec::new();
                if let Some(text) = content.as_ref().filter(|text| !text.is_empty()) {
                    parts.push(json!({"text":text}));
                }
                if let Some(calls) = tool_calls {
                    for call in calls {
                        call_names.insert(call.id.clone(), call.function.name.clone());
                        let args = serde_json::from_str(&call.function.arguments)
                            .unwrap_or_else(|_| json!({}));
                        parts.push(json!({"functionCall":{"name":call.function.name,"args":args}}));
                    }
                }
                if !parts.is_empty() {
                    contents.push(json!({"role":"model","parts":parts}));
                }
            }
            Message::Tool {
                tool_call_id,
                name,
                content,
            } => {
                let name = name
                    .clone()
                    .or_else(|| call_names.get(tool_call_id).cloned())
                    .unwrap_or_else(|| "unknown".into());
                contents.push(json!({"role":"function","parts":[{"functionResponse":{"name":name,"response":{"result":content}}}]}));
            }
        }
    }
    let system = (!system.is_empty()).then(|| json!({"parts":system}));
    (contents, system)
}

fn content_text(content: &Content) -> String {
    match content {
        Content::Text(text) => text.clone(),
        Content::Multimodal(parts) => parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(text.clone()),
                ContentPart::Image { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn tools_to_genai(tools: &[Value]) -> Vec<Value> {
    let declarations = tools.iter().filter_map(|tool| tool.get("function")).map(|function| {
        let mut declaration = json!({"name":function.get("name").cloned().unwrap_or(json!("")),"description":function.get("description").cloned().unwrap_or(json!(""))});
        if let Some(parameters) = function.get("parameters") { declaration["parameters"] = parameters.clone(); }
        declaration
    }).collect::<Vec<_>>();
    if declarations.is_empty() {
        Vec::new()
    } else {
        vec![json!({"functionDeclarations":declarations})]
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
    fn normalizes_google_reasoning_text_tool_usage_and_finish() {
        let events = decode_google_chunk(
            &json!({"response":{"usageMetadata":{"promptTokenCount":2},"candidates":[{"content":{"parts":[{"text":"think","thought":true},{"text":"answer"},{"functionCall":{"name":"read_file","args":{"path":"a"}}}]},"finishReason":"STOP"}]}}),
        );
        assert!(events.contains(&NormalizedStreamEvent::ReasoningDelta("think".into())));
        assert!(events.contains(&NormalizedStreamEvent::TextDelta("answer".into())));
        assert!(events.iter().any(|event| matches!(event, NormalizedStreamEvent::ToolCallStart(delta) if delta.name.as_deref() == Some("read_file"))));
        assert!(events.contains(&NormalizedStreamEvent::FinishReason("STOP".into())));
    }
}
