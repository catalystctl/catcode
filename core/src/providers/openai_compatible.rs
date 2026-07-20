use super::adapter::{
    normalize_http_error, BuiltProviderRequest, ProviderAdapter, ProviderError, ProviderProtocol,
    ProviderRequest,
};
use super::capabilities::ProviderCapabilities;
use super::streaming::{decode_openai_chunk, NormalizedStreamEvent};
use crate::message::Message;
use serde_json::{json, Value};

pub struct OpenAiCompatibleAdapter;

impl ProviderAdapter for OpenAiCompatibleAdapter {
    fn id(&self) -> &'static str {
        "openai_compatible"
    }

    fn protocol(&self) -> ProviderProtocol {
        ProviderProtocol::OpenAiChat
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
        let mut tools = input.tools.to_vec();
        tools.sort_by(|a, b| tool_name(a).cmp(tool_name(b)));
        let mut body = json!({
            "model": input.model,
            "messages": Message::to_openai_messages(input.messages),
            "tools": tools,
            "tool_choice": "auto",
            "stream": true,
            "stream_options": { "include_usage": true },
        });
        if input
            .tools
            .iter()
            .any(|tool| tool_name(tool) == "goal_write_plan")
        {
            body["tool_choice"] = json!({
                "type": "function",
                "function": { "name": "goal_write_plan" }
            });
        }

        let supports_reasoning = crate::provider::is_umans(&input.provider.base_url)
            || crate::provider::is_cursor_bridge(&input.provider.base_url);
        let mut notices = Vec::new();
        if supports_reasoning {
            let resolved =
                crate::provider::resolve_effort(input.reasoning_effort, input.thinking_levels);
            if resolved != input.reasoning_effort {
                notices.push(format!(
                    "reasoning effort '{}' not supported by model '{}'; using '{}'",
                    input.reasoning_effort, input.model, resolved
                ));
            }
            body["reasoning_effort"] = json!(resolved);
        }

        Ok(BuiltProviderRequest {
            url: format!(
                "{}/chat/completions",
                input.provider.base_url.trim_end_matches('/')
            ),
            body,
            notices,
        })
    }

    fn decode_stream_event(&self, value: &Value) -> Vec<NormalizedStreamEvent> {
        decode_openai_chunk(value)
    }

    fn normalize_error(&self, status: Option<u16>, body: &str) -> ProviderError {
        normalize_http_error(status, body)
    }
}

fn tool_name(tool: &Value) -> &str {
    tool.get("function")
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ProviderKind, ResolvedProvider};

    fn provider(base_url: &str) -> ResolvedProvider {
        ResolvedProvider {
            name: "test".into(),
            kind: ProviderKind::OpenAI,
            base_url: base_url.into(),
            api_key: None,
            headers: Vec::new(),
            oauth: false,
        }
    }

    #[test]
    fn request_is_stable_and_gates_nonstandard_reasoning() {
        let provider = provider("https://example.com/v1");
        let tools = vec![
            json!({"function":{"name":"z_tool"}}),
            json!({"function":{"name":"a_tool"}}),
        ];
        let request = ProviderRequest {
            provider: &provider,
            model: "model",
            messages: &[],
            tools: &tools,
            reasoning_effort: "high",
            thinking_levels: &[],
            max_tokens: 100,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert_eq!(built.url, "https://example.com/v1/chat/completions");
        assert_eq!(built.body["tools"][0]["function"]["name"], "a_tool");
        assert!(built.body.get("reasoning_effort").is_none());
    }
}
