use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub reasoning: bool,
    pub context_window: u32,
    pub max_tokens: u32,
    /// Reasoning/thinking levels the model advertises (e.g. ["low","medium","high"]).
    /// Populated from /models/info when the endpoint provides them; empty means the
    /// model declares no specific levels and any effort string is passed through.
    #[serde(default)]
    pub thinking_levels: Vec<String>,
    /// Whether the model accepts image (vision) inputs. Populated from
    /// /models/info `capabilities.supports_vision` (true/false/"via-handoff";
    /// only boolean true counts as native client-side vision); false otherwise.
    /// Drives the vision-handoff (pre_turn plugin) routing.
    #[serde(default)]
    pub vision: bool,
    /// The provider name that owns this model (e.g. "openai", "gemini",
    /// "anthropic"). Populated by the aggregation layer so a turn can be routed
    /// to the correct endpoint per-model when multiple providers are logged in.
    /// Empty for legacy single-provider models (routes to the active provider).
    #[serde(default)]
    pub provider: String,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct ClientInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}
