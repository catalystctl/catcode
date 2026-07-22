use super::adapter::{ProviderAdapter, ProviderProtocol};
use super::anthropic_compatible::AnthropicCompatibleAdapter;
use super::codex_responses::CodexResponsesAdapter;
use super::google_code_assist::GoogleCodeAssistAdapter;
use super::openai_compatible::OpenAiCompatibleAdapter;
use crate::config::{ProviderKind, ResolvedProvider};

static OPENAI: OpenAiCompatibleAdapter = OpenAiCompatibleAdapter;
static ANTHROPIC: AnthropicCompatibleAdapter = AnthropicCompatibleAdapter;
static CODEX: CodexResponsesAdapter = CodexResponsesAdapter;
static GOOGLE: GoogleCodeAssistAdapter = GoogleCodeAssistAdapter;

/// Resolve the wire adapter once at the provider boundary. Endpoint-specific
/// protocols that are not OpenAI chat-completions are named explicitly so the
/// turn dispatcher does not spread URL heuristics through unrelated code.
pub fn adapter_for(provider: &ResolvedProvider) -> &'static dyn ProviderAdapter {
    if provider.kind == ProviderKind::Anthropic {
        &ANTHROPIC
    } else if crate::provider::is_code_assist_endpoint(&provider.base_url) {
        &GOOGLE
    } else if crate::provider::is_codex_endpoint(&provider.base_url) {
        &CODEX
    } else {
        &OPENAI
    }
}

pub fn protocol_for(provider: &ResolvedProvider) -> ProviderProtocol {
    if provider.kind == ProviderKind::Anthropic {
        ProviderProtocol::AnthropicMessages
    } else {
        adapter_for(provider).protocol()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(kind: ProviderKind, base_url: &str) -> ResolvedProvider {
        ResolvedProvider {
            name: "test".into(),
            kind,
            base_url: base_url.into(),
            api_key: None,
            headers: Vec::new(),
            oauth: false,
            context_window: None,
            models_override: Vec::new(),
        }
    }

    #[test]
    fn resolves_protocols_without_turn_loop_branching() {
        assert_eq!(
            protocol_for(&provider(
                ProviderKind::Anthropic,
                "https://api.anthropic.com/v1"
            )),
            ProviderProtocol::AnthropicMessages
        );
        assert_eq!(
            protocol_for(&provider(
                ProviderKind::OpenAI,
                "https://chatgpt.com/backend-api/codex"
            )),
            ProviderProtocol::CodexResponses
        );
        assert_eq!(
            protocol_for(&provider(ProviderKind::OpenAI, "https://example.com/v1")),
            ProviderProtocol::OpenAiChat
        );
    }
}
