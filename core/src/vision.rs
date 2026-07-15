// Vision handoff configuration: user-curated vision-capable models + a preferred
// handoff target, persisted to .catalyst-code/vision.json. The core merges the
// curated set with the endpoint's `capabilities.vision` flag when building the
// pre_turn hook context, ranks the cheapest same-provider vision model, and
// passes `recommended_vision_model` + `vision_model` to the vision-handoff plugin.
use crate::protocol::ModelInfo;
use serde_json::json;
use std::path::Path;

/// User-curated vision configuration.
///
/// - `enabled` — auto handoff on image turns when the active model lacks vision
///   (defaults **true** / recommended ON).
/// - `vision_models` — declares which models can handle images (merged with the
///   endpoint's flag).
/// - `vision_model` — preferred handoff target (None = cheapest same-provider).
#[derive(Clone, Debug)]
pub struct VisionConfig {
    pub enabled: bool,
    pub vision_models: Vec<String>,
    pub vision_model: Option<String>,
}

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            vision_models: Vec::new(),
            vision_model: None,
        }
    }
}

const FILENAME: &str = "vision.json";

fn path(workspace: &Path) -> std::path::PathBuf {
    workspace.join(".catalyst-code").join(FILENAME)
}

impl VisionConfig {
    /// Load from <workspace>/.catalyst-code/vision.json; default if absent/unreadable.
    /// Missing `enabled` key defaults to true (recommended ON).
    pub fn load(workspace: &Path) -> Self {
        let p = path(workspace);
        let Ok(content) = std::fs::read_to_string(&p) else {
            return Self::default();
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) else {
            return Self::default();
        };
        let enabled = v
            .get("enabled")
            .and_then(|b| b.as_bool())
            .unwrap_or(true);
        let vision_models = v
            .get("vision_models")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let vision_model = v
            .get("vision_model")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        Self {
            enabled,
            vision_models,
            vision_model,
        }
    }

    /// Persist to <workspace>/.catalyst-code/vision.json (best-effort).
    pub fn save(&self, workspace: &Path) {
        let p = path(workspace);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let body = json!({
            "enabled": self.enabled,
            "vision_models": self.vision_models,
            "vision_model": self.vision_model,
        });
        let _ = std::fs::write(&p, serde_json::to_string_pretty(&body).unwrap_or_default());
    }

    /// True if `id` is in the user-curated vision-capable list.
    pub fn has_vision(&self, id: &str) -> bool {
        self.vision_models.iter().any(|m| m == id)
    }
}

/// Coarse cost rank for picking the cheapest vision model within a provider.
/// Lower is cheaper. Uses id heuristics when real prices are unavailable.
pub fn vision_cost_rank(model_id: &str) -> i32 {
    let l = model_id.to_ascii_lowercase();
    // Expensive first when substrings collide (e.g. "ultra" contains "lite").
    if l.contains("opus") || l.contains("ultra") || l.contains("-max") || l.ends_with("max") {
        return 80;
    }
    if l.contains("o1") || l.contains("o3") {
        return 80;
    }
    // Cheapest / small
    if l.contains("nano")
        || l.contains("haiku")
        || l.contains("mini")
        || l.contains("flash-lite")
        || l.contains("flash_lite")
    {
        return 10;
    }
    if l.contains("flash")
        || l.contains("-lite")
        || l.contains("_lite")
        || l.contains("small")
        || l.contains("fast")
    {
        return 20;
    }
    // Mid
    if l.contains("sonnet") || l.contains("codex") || l.contains("gpt-4o") {
        return 40;
    }
    if l.contains("-pro") || l.contains("pro-") || l.ends_with("pro") || l.contains("medium") {
        return 50;
    }
    60 // unknown mid-high
}

/// Whether `candidate` is vision-capable given endpoint flag ∪ curated list.
pub fn model_has_vision(m: &ModelInfo, vc: &VisionConfig) -> bool {
    m.vision || vc.has_vision(m.id.as_str())
}

/// Pick the handoff target for an image turn on `active_model_id`.
///
/// Precedence:
/// 1. Preferred `vision_model` if set, known, and vision-capable
/// 2. Cheapest same-provider vision candidate (by [`vision_cost_rank`], then id)
///
/// Returns None when no suitable candidate exists.
pub fn recommend_vision_model(
    active_model_id: &str,
    models: &[ModelInfo],
    vc: &VisionConfig,
) -> Option<String> {
    let active_provider = models
        .iter()
        .find(|m| m.id == active_model_id)
        .map(|m| m.provider.as_str())
        .unwrap_or("");

    if let Some(pref) = vc.vision_model.as_ref() {
        if let Some(m) = models.iter().find(|m| m.id == *pref) {
            if model_has_vision(m, vc) && providers_match(active_provider, &m.provider) {
                return Some(pref.clone());
            }
            // Preferred pin may intentionally cross providers when the user set it;
            // still allow if vision-capable and known.
            if model_has_vision(m, vc) {
                return Some(pref.clone());
            }
        }
    }

    let mut candidates: Vec<&ModelInfo> = models
        .iter()
        .filter(|m| m.id != active_model_id)
        .filter(|m| model_has_vision(m, vc))
        .filter(|m| providers_match(active_provider, &m.provider))
        .collect();

    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by(|a, b| {
        vision_cost_rank(&a.id)
            .cmp(&vision_cost_rank(&b.id))
            .then_with(|| a.id.cmp(&b.id))
    });
    candidates.first().map(|m| m.id.clone())
}

fn providers_match(a: &str, b: &str) -> bool {
    a == b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(unique: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("umans_vision_test_{unique}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mi(id: &str, provider: &str, vision: bool) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            name: id.into(),
            provider: provider.into(),
            vision,
            ..Default::default()
        }
    }

    #[test]
    fn load_missing_file_is_default_enabled() {
        let dir = tmp("missing");
        let cfg = VisionConfig::load(&dir);
        assert!(cfg.enabled);
        assert!(cfg.vision_models.is_empty());
        assert!(cfg.vision_model.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_then_load_roundtrip_with_enabled() {
        let dir = tmp("roundtrip");
        let cfg = VisionConfig {
            enabled: false,
            vision_models: vec!["umans-kimi-k2.6".into(), "umans-coder".into()],
            vision_model: Some("umans-kimi-k2.6".into()),
        };
        cfg.save(&dir);
        let loaded = VisionConfig::load(&dir);
        assert!(!loaded.enabled);
        assert_eq!(loaded.vision_models, cfg.vision_models);
        assert_eq!(loaded.vision_model, Some("umans-kimi-k2.6".to_string()));
        assert!(loaded.has_vision("umans-coder"));
        assert!(!loaded.has_vision("umans-glm-5.2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_enabled_key_defaults_true() {
        let dir = tmp("no_enabled");
        std::fs::create_dir_all(dir.join(".catalyst-code")).unwrap();
        std::fs::write(
            dir.join(".catalyst-code").join("vision.json"),
            r#"{"vision_models":["x"],"vision_model":""}"#,
        )
        .unwrap();
        let cfg = VisionConfig::load(&dir);
        assert!(cfg.enabled);
        assert_eq!(cfg.vision_models, vec!["x".to_string()]);
        assert!(cfg.vision_model.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_vision_model_becomes_none() {
        let dir = tmp("empty_vm");
        std::fs::create_dir_all(dir.join(".catalyst-code")).unwrap();
        std::fs::write(
            dir.join(".catalyst-code").join("vision.json"),
            r#"{"enabled":true,"vision_models":["x"],"vision_model":""}"#,
        )
        .unwrap();
        let cfg = VisionConfig::load(&dir);
        assert_eq!(cfg.vision_models, vec!["x".to_string()]);
        assert!(cfg.vision_model.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_file_is_default() {
        let dir = tmp("malformed");
        std::fs::create_dir_all(dir.join(".catalyst-code")).unwrap();
        std::fs::write(
            dir.join(".catalyst-code").join("vision.json"),
            "not json {{{",
        )
        .unwrap();
        let cfg = VisionConfig::load(&dir);
        assert!(cfg.enabled);
        assert!(cfg.vision_models.is_empty());
        assert!(cfg.vision_model.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cost_rank_prefers_haiku_over_opus() {
        assert!(vision_cost_rank("claude-haiku-4") < vision_cost_rank("claude-opus-4"));
        assert!(vision_cost_rank("gpt-4o-mini") < vision_cost_rank("gpt-4o"));
        // "flash" before "ultra"/"max" families; avoid ids where substrings collide
        // (e.g. accidental "pro" in longer names).
        assert!(vision_cost_rank("gemini-flash") < vision_cost_rank("gemini-ultra"));
    }

    #[test]
    fn recommend_cheapest_same_provider() {
        let models = vec![
            mi("coder", "umans", false),
            mi("opus-vision", "umans", true),
            mi("haiku-vision", "umans", true),
            mi("flash-other", "openai", true),
        ];
        let vc = VisionConfig::default();
        let got = recommend_vision_model("coder", &models, &vc);
        assert_eq!(got.as_deref(), Some("haiku-vision"));
    }

    #[test]
    fn recommend_preferred_overrides_cheapest() {
        let models = vec![
            mi("coder", "umans", false),
            mi("opus-vision", "umans", true),
            mi("haiku-vision", "umans", true),
        ];
        let vc = VisionConfig {
            enabled: true,
            vision_models: vec![],
            vision_model: Some("opus-vision".into()),
        };
        let got = recommend_vision_model("coder", &models, &vc);
        assert_eq!(got.as_deref(), Some("opus-vision"));
    }

    #[test]
    fn recommend_none_when_no_same_provider_vision() {
        let models = vec![
            mi("coder", "umans", false),
            mi("flash", "openai", true),
        ];
        let vc = VisionConfig::default();
        assert!(recommend_vision_model("coder", &models, &vc).is_none());
    }

    #[test]
    fn recommend_uses_curated_vision_list() {
        let models = vec![
            mi("coder", "umans", false),
            mi("special", "umans", false), // not endpoint-vision, but curated
        ];
        let vc = VisionConfig {
            enabled: true,
            vision_models: vec!["special".into()],
            vision_model: None,
        };
        let got = recommend_vision_model("coder", &models, &vc);
        assert_eq!(got.as_deref(), Some("special"));
    }
}
