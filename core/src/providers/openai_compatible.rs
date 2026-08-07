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
        let kimi = crate::provider::is_kimi(&input.provider.base_url);
        let deepseek = crate::provider::is_deepseek(&input.provider.base_url);
        let zhipu = crate::provider::is_zhipu(&input.provider.base_url);
        let minimax_host = crate::provider::is_minimax(&input.provider.base_url);
        let model_l = input.model.to_ascii_lowercase();
        let minimax_model = model_l.contains("minimax");
        let minimax = minimax_host || minimax_model;
        // Non-standard request fields (reasoning_effort / thinking / reasoning_content
        // replay) are host- or model-gated. Vanilla OpenAI-compatible servers reject
        // them with 400, so never put them on the wire unless the host/model is known.
        // MiniMax-by-model covers proxy gateways (e.g. ai.karutoil.site) that front
        // MiniMax-M3 but are not api.minimax.io.
        let supports_reasoning = crate::provider::is_umans(&input.provider.base_url)
            || crate::provider::is_cursor_bridge(&input.provider.base_url)
            || kimi
            || deepseek
            || zhipu
            || minimax;

        let mut messages = Message::to_openai_messages(input.messages);
        if !supports_reasoning {
            for message in &mut messages {
                if let Some(obj) = message.as_object_mut() {
                    obj.remove("reasoning_content");
                }
            }
        } else if minimax_model && !minimax_host {
            // Proxy MiniMax: vendors that ignore reasoning_split expect prior
            // thinking re-embedded as <think>…</think> inside assistant content
            // for multi-turn tool continuity (official MiniMax multi-turn note).
            for message in &mut messages {
                let Some(obj) = message.as_object_mut() else {
                    continue;
                };
                if obj.get("role").and_then(|r| r.as_str()) != Some("assistant") {
                    continue;
                }
                let Some(thinking) = obj
                    .remove("reasoning_content")
                    .and_then(|v| v.as_str().map(str::to_string))
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                let content = obj
                    .get("content")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                if content.contains("<think>") {
                    // Already tagged (e.g. raw session import) — keep as-is.
                    obj.insert("content".into(), json!(content));
                } else if content.is_empty() {
                    obj.insert(
                        "content".into(),
                        json!(format!("<think>\n{thinking}\n</think>\n")),
                    );
                } else {
                    obj.insert(
                        "content".into(),
                        json!(format!("<think>\n{thinking}\n</think>\n{content}")),
                    );
                }
            }
        }

        let mut body = json!({
            "model": input.model,
            "messages": messages,
            "stream": true,
        });
        // stream_options is OpenAI-native; Z.ai's OpenAPI schema does not list it
        // and may drop usage or reject the field on strict gateways.
        if !zhipu {
            body["stream_options"] = json!({ "include_usage": true });
        }

        if input.max_tokens > 0 {
            if kimi || minimax {
                // Official Kimi/MiniMax OpenAPI prefer max_completion_tokens.
                body["max_completion_tokens"] = json!(input.max_tokens);
            } else {
                body["max_tokens"] = json!(input.max_tokens);
            }
        }

        if !input.tools.is_empty() {
            let mut tools = input.tools.to_vec();
            tools.sort_by(|a, b| tool_name(a).cmp(tool_name(b)));
            body["tools"] = json!(tools);
            // Z.ai documents tool_choice as auto-only. Named-function forcing
            // is OpenAI-native and can 400 on first-party Zhipu endpoints.
            if !zhipu
                && input
                    .tools
                    .iter()
                    .any(|tool| tool_name(tool) == "goal_write_plan")
            {
                body["tool_choice"] = json!({
                    "type": "function",
                    "function": { "name": "goal_write_plan" }
                });
            } else {
                body["tool_choice"] = json!("auto");
            }
        }

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
            let off = input.reasoning_effort.eq_ignore_ascii_case("none")
                || input.reasoning_effort.is_empty();
            if kimi {
                // Official Kimi platform is model-family-specific:
                // K3 uses reasoning_effort only; K2.7-code always thinks;
                // K2.6 uses thinking toggle.
                if model_l.contains("k3") {
                    let effort = if off {
                        "low".to_string()
                    } else if resolved.eq_ignore_ascii_case("medium") {
                        "high".to_string()
                    } else {
                        resolved
                    };
                    body["reasoning_effort"] = json!(effort);
                } else if model_l.contains("k2.7-code") || model_l.contains("for-coding") {
                    body["thinking"] = json!({ "type": "enabled", "keep": "all" });
                } else if off {
                    body["thinking"] = json!({ "type": "disabled" });
                } else {
                    body["thinking"] = json!({ "type": "enabled" });
                }
            } else if deepseek {
                if off {
                    body["thinking"] = json!({ "type": "disabled" });
                } else {
                    body["reasoning_effort"] = json!(resolved);
                    body["thinking"] = json!({ "type": "enabled" });
                }
            } else if zhipu {
                if off {
                    body["thinking"] = json!({ "type": "disabled" });
                } else {
                    body["thinking"] = json!({ "type": "enabled" });
                    if model_l.contains("glm-5.2") && !resolved.is_empty() {
                        body["reasoning_effort"] = json!(resolved);
                    }
                }
            } else if minimax {
                // MiniMax OpenAI wire: thinking adaptive/disabled only.
                // reasoning_split is first-party only — many proxies ignore or
                // 400 on it; think-tag demux handles unsplit content streams.
                body["thinking"] = json!({
                    "type": if off { "disabled" } else { "adaptive" }
                });
                if !off && minimax_host {
                    body["reasoning_split"] = json!(true);
                }
            } else if !resolved.is_empty() {
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
        assert_eq!(built.body["max_tokens"], 100);
    }

    #[test]
    fn max_tokens_zero_is_omitted_not_sent() {
        // Never put max_tokens:0 on the wire — some proxies treat it as
        // "generate nothing". Omitting leaves the server default (uncapped by us).
        let provider = provider("https://example.com/v1");
        let request = ProviderRequest {
            provider: &provider,
            model: "model",
            messages: &[],
            tools: &[],
            reasoning_effort: "none",
            thinking_levels: &[],
            max_tokens: 0,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert!(
            built.body.get("max_tokens").is_none(),
            "max_tokens:0 must be omitted, got {:?}",
            built.body.get("max_tokens")
        );
    }

    #[test]
    fn empty_tools_and_tool_choice_are_omitted() {
        // tools:[] + tool_choice:"auto" 400 on strict OpenAI-compatible servers.
        // When the harness has no tools, omit both fields entirely.
        let provider = provider("https://example.com/v1");
        let request = ProviderRequest {
            provider: &provider,
            model: "model",
            messages: &[],
            tools: &[],
            reasoning_effort: "none",
            thinking_levels: &[],
            max_tokens: 100,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert!(
            built.body.get("tools").is_none(),
            "empty tools must be omitted, got {:?}",
            built.body.get("tools")
        );
        assert!(
            built.body.get("tool_choice").is_none(),
            "tool_choice without tools must be omitted, got {:?}",
            built.body.get("tool_choice")
        );
        // stream_options.include_usage stays — we still want usage accounting.
        assert_eq!(built.body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn non_empty_tools_still_send_sorted_tools_and_auto_choice() {
        let provider = provider("https://example.com/v1");
        let tools = vec![
            json!({"type":"function","function":{"name":"z_tool"}}),
            json!({"type":"function","function":{"name":"a_tool"}}),
        ];
        let request = ProviderRequest {
            provider: &provider,
            model: "model",
            messages: &[],
            tools: &tools,
            reasoning_effort: "none",
            thinking_levels: &[],
            max_tokens: 100,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert_eq!(built.body["tools"][0]["function"]["name"], "a_tool");
        assert_eq!(built.body["tools"][1]["function"]["name"], "z_tool");
        assert_eq!(built.body["tool_choice"], "auto");
    }

    #[test]
    fn goal_write_plan_forces_tool_choice_when_tools_present() {
        let provider = provider("https://example.com/v1");
        let tools = vec![json!({
            "type": "function",
            "function": { "name": "goal_write_plan" }
        })];
        let request = ProviderRequest {
            provider: &provider,
            model: "model",
            messages: &[],
            tools: &tools,
            reasoning_effort: "none",
            thinking_levels: &[],
            max_tokens: 100,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert_eq!(built.body["tool_choice"]["type"], "function");
        assert_eq!(
            built.body["tool_choice"]["function"]["name"],
            "goal_write_plan"
        );
    }

    #[test]
    fn vanilla_endpoint_strips_persisted_reasoning_content() {
        // A session that previously hit DeepSeek/Kimi/Umans may have assistant
        // messages with reasoning_content on disk. Replaying them to a vanilla
        // OpenAI-compatible proxy must not put that non-standard field on wire.
        let provider = provider("https://example.com/v1");
        let messages = vec![Message::try_from(&json!({
            "role": "assistant",
            "content": "ok",
            "reasoning_content": "secret thoughts from a prior vendor"
        }))
        .unwrap()];
        let request = ProviderRequest {
            provider: &provider,
            model: "model",
            messages: &messages,
            tools: &[],
            reasoning_effort: "none",
            thinking_levels: &[],
            max_tokens: 100,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert_eq!(built.body["messages"][0]["content"], "ok");
        assert!(
            built.body["messages"][0].get("reasoning_content").is_none(),
            "reasoning_content must be stripped for vanilla hosts, got {:?}",
            built.body["messages"][0]
        );
    }

    #[test]
    fn deepseek_endpoint_keeps_persisted_reasoning_content() {
        let provider = deepseek_provider();
        let messages = vec![Message::try_from(&json!({
            "role": "assistant",
            "content": "ok",
            "reasoning_content": "keep me"
        }))
        .unwrap()];
        let request = ProviderRequest {
            provider: &provider,
            model: "deepseek-v4-flash",
            messages: &messages,
            tools: &[],
            reasoning_effort: "none",
            thinking_levels: &[],
            max_tokens: 100,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert_eq!(built.body["messages"][0]["reasoning_content"], "keep me");
    }

    fn kimi_provider() -> ResolvedProvider {
        provider("https://api.kimi.com/coding/v1")
    }

    #[test]
    fn kimi_for_coding_always_enables_thinking_keep_all() {
        // Official Kimi coding models always think; effort is not a disable switch.
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
        assert!(built.body.get("reasoning_effort").is_none());
        assert_eq!(
            built.body["thinking"],
            json!({ "type": "enabled", "keep": "all" })
        );
        assert_eq!(built.body["max_completion_tokens"], 100);
    }

    #[test]
    fn kimi_for_coding_none_effort_still_keeps_thinking() {
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
        assert_eq!(
            built.body["thinking"],
            json!({ "type": "enabled", "keep": "all" })
        );
    }

    #[test]
    fn kimi_k3_sends_reasoning_effort_only() {
        let levels = ["low".to_string(), "high".to_string(), "max".to_string()];
        let provider = kimi_provider();
        let request = ProviderRequest {
            provider: &provider,
            model: "kimi-k3",
            messages: &[],
            tools: &[],
            reasoning_effort: "high",
            thinking_levels: &levels,
            max_tokens: 200,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert_eq!(built.body["reasoning_effort"], json!("high"));
        assert!(built.body.get("thinking").is_none());
        assert_eq!(built.body["max_completion_tokens"], 200);
    }

    #[test]
    fn minimax_openai_sends_adaptive_thinking_and_reasoning_split() {
        let levels = ["low".to_string(), "medium".to_string(), "high".to_string()];
        let provider = provider("https://api.minimax.io/v1");
        let request = ProviderRequest {
            provider: &provider,
            model: "MiniMax-M3",
            messages: &[],
            tools: &[],
            reasoning_effort: "high",
            thinking_levels: &levels,
            max_tokens: 500,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert_eq!(built.body["thinking"], json!({ "type": "adaptive" }));
        assert_eq!(built.body["reasoning_split"], true);
        assert_eq!(built.body["max_completion_tokens"], 500);
        assert!(built.body.get("reasoning_effort").is_none());
    }

    #[test]
    fn minimax_openai_disables_thinking_when_effort_none() {
        let levels = ["low".to_string(), "medium".to_string(), "high".to_string()];
        let provider = provider("https://api.minimax.io/v1");
        let request = ProviderRequest {
            provider: &provider,
            model: "MiniMax-M3",
            messages: &[],
            tools: &[],
            reasoning_effort: "none",
            thinking_levels: &levels,
            max_tokens: 100,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert_eq!(built.body["thinking"], json!({ "type": "disabled" }));
        assert!(built.body.get("reasoning_split").is_none());
    }

    #[test]
    fn minimax_proxy_model_enables_thinking_without_reasoning_split() {
        // Proxy gateway hosting minimax-m3 (not api.minimax.io): still send
        // adaptive thinking, but never reasoning_split (proxies often 400/ignore).
        let levels = ["low".to_string(), "medium".to_string(), "high".to_string()];
        let provider = provider("https://ai.example.proxy/v1");
        let request = ProviderRequest {
            provider: &provider,
            model: "minimax-m3",
            messages: &[],
            tools: &[],
            reasoning_effort: "high",
            thinking_levels: &levels,
            max_tokens: 500,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert_eq!(built.body["thinking"], json!({ "type": "adaptive" }));
        assert!(built.body.get("reasoning_split").is_none());
        assert_eq!(built.body["max_completion_tokens"], 500);
    }

    #[test]
    fn minimax_proxy_reembeds_reasoning_into_content_for_replay() {
        // Multi-turn tool continuity on proxies that only understand <think> tags.
        let levels = ["high".to_string()];
        let provider = provider("https://ai.example.proxy/v1");
        let messages = vec![Message::try_from(&json!({
            "role": "assistant",
            "content": "visible answer",
            "reasoning_content": "hidden chain"
        }))
        .unwrap()];
        let request = ProviderRequest {
            provider: &provider,
            model: "minimax-m3",
            messages: &messages,
            tools: &[],
            reasoning_effort: "high",
            thinking_levels: &levels,
            max_tokens: 100,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        let content = built.body["messages"][0]["content"].as_str().unwrap();
        assert!(content.starts_with("<think>\nhidden chain\n</think>\n"));
        assert!(content.ends_with("visible answer"));
        assert!(
            built.body["messages"][0]
                .get("reasoning_content")
                .is_none(),
            "proxy path must not send bare reasoning_content"
        );
    }


    #[test]
    fn zhipu_omits_stream_options_and_enables_thinking() {
        let levels = ["high".to_string(), "max".to_string()];
        let provider = provider("https://open.bigmodel.cn/api/paas/v4");
        let request = ProviderRequest {
            provider: &provider,
            model: "glm-5.2",
            messages: &[],
            tools: &[],
            reasoning_effort: "max",
            thinking_levels: &levels,
            max_tokens: 100,
        };
        let built = OpenAiCompatibleAdapter.build_request(&request).unwrap();
        assert!(built.body.get("stream_options").is_none());
        assert_eq!(built.body["thinking"], json!({ "type": "enabled" }));
        assert_eq!(built.body["reasoning_effort"], json!("max"));
        assert_eq!(built.body["max_tokens"], 100);
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
