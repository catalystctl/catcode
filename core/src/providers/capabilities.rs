use serde::Serialize;

/// Wire-level features advertised by a provider adapter. These flags describe
/// what the adapter can encode/decode; model-specific capabilities remain on
/// `ModelInfo`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ProviderCapabilities {
    pub streaming: bool,
    pub tools: bool,
    pub parallel_tools: bool,
    pub reasoning: bool,
    pub vision: bool,
    pub usage: bool,
    pub model_discovery: bool,
}
