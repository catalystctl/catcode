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

        let kimi = crate::provider::is_kimi(&input.provider.base_url);
        let deepseek = crate::provider::is_deepseek(&input.provider.base_url);
        let supports_reasoning = crate::provider::is_umans(&input.provider.base_url)
            || crate::provider::is_cursor_bridge(&input.provider.base_url)
            || kimi
            || deepseek;
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
            if kimi || deepseek {
                // Kimi and DeepSeek use a DUAL thinking mechanism: top-level
                // `reasoning_effort` AND `thinking: {type: enabled|disabled}`
                // are sent together. Gate on/off via the ORIGINAL requested
                // effort — `resolve_effort` clamps "none" up to a supported
                // level for leveled models, so the resolved value can't tell
                // us the user wants thinking OFF.
                let off = input.reasoning_effort.eq_ignore_ascii_case("none")
                    || input.reasoning_effort.is_empty();
                if off {
                    body["thinking"] = json!({ "type": "disabled" });
                } else {
                    body["reasoning_effort"] = json!(resolved);
                    body["thinking"] = json!({ "type": "enabled" });
                }
            } else {
                body["reasoning_effort"] = json!(resolved);
            }
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
            context_window: None,
            models_override: Vec::new(),
            models_endpoint: None,
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

    fn kimi_provider() -> ResolvedProvider {
        provider("https://api.kimi.com/coding/v1")
    }

    #[test]
    fn kimi_injects_dual_thinking_when_effort_set() {
        // Kimi's dual mechanism: reasoning_effort AND thinking.type together.
        let levels = ["low".to_string(), "medium".to_string(), "high".to_string()];
        let provider = kimi_provider();
        let request = ProviderRequest {
            provider: &provider,
            model: "kimi-for-coding",
            messages: &[],
            tools: &[],
            reasoning_effort: "high",
            thinking_levels: &levels,
            max_tokens: 100,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert_eq!(built.body["reasoning_effort"], json!("high"));
        assert_eq!(built.body["thinking"], json!({ "type": "enabled" }));
    }

    #[test]
    fn kimi_disables_thinking_when_effort_none() {
        // resolve_effort clamps "none" up to a supported level for leveled
        // models, so the gate uses the ORIGINAL effort: "none" must send
        // thinking.type=disabled and NOT send reasoning_effort.
        let levels = ["low".to_string(), "medium".to_string(), "high".to_string()];
        let provider = kimi_provider();
        let request = ProviderRequest {
            provider: &provider,
            model: "kimi-for-coding",
            messages: &[],
            tools: &[],
            reasoning_effort: "none",
            thinking_levels: &levels,
            max_tokens: 100,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert!(built.body.get("reasoning_effort").is_none());
        assert_eq!(built.body["thinking"], json!({ "type": "disabled" }));
    }

    #[test]
    fn non_vendor_openai_endpoint_sends_no_thinking_field() {
        // A vanilla OpenAI-compatible endpoint must not get vendor-specific
        // thinking fields (they would 400) nor reasoning_effort.
        let provider = provider("https://example.com/v1");
        let request = ProviderRequest {
            provider: &provider,
            model: "model",
            messages: &[],
            tools: &[],
            reasoning_effort: "high",
            thinking_levels: &[],
            max_tokens: 100,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert!(built.body.get("reasoning_effort").is_none());
        assert!(built.body.get("thinking").is_none());
    }

    fn deepseek_provider() -> ResolvedProvider {
        provider("https://api.deepseek.com")
    }

    #[test]
    fn deepseek_injects_vendor_thinking_fields_when_effort_set() {
        let levels = ["high".to_string(), "max".to_string()];
        let provider = deepseek_provider();
        let request = ProviderRequest {
            provider: &provider,
            model: "deepseek-v4-flash",
            messages: &[],
            tools: &[],
            reasoning_effort: "max",
            thinking_levels: &levels,
            max_tokens: 100,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert_eq!(built.body["reasoning_effort"], json!("max"));
        assert_eq!(built.body["thinking"], json!({ "type": "enabled" }));
    }

    #[test]
    fn deepseek_disables_thinking_when_effort_none() {
        let levels = ["high".to_string(), "max".to_string()];
        let provider = deepseek_provider();
        let request = ProviderRequest {
            provider: &provider,
            model: "deepseek-v4-flash",
            messages: &[],
            tools: &[],
            reasoning_effort: "none",
            thinking_levels: &levels,
            max_tokens: 100,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert!(built.body.get("reasoning_effort").is_none());
        assert_eq!(built.body["thinking"], json!({ "type": "disabled" }));
    }
}
