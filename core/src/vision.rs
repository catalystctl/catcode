// Vision handoff configuration: user-curated vision-capable models + a preferred
// handoff target, persisted to .catalyst-code/vision.json. The core merges the
// curated set with the endpoint's `capabilities.vision` flag when building the
// pre_turn hook context, and passes `vision_model` as the preferred target the
// vision-handoff plugin should route image-bearing turns to.
use serde_json::json;
use std::path::Path;

/// User-curated vision configuration. `vision_models` declares which models can
/// handle images (merged with the endpoint's flag); `vision_model` is the
/// preferred handoff target (None = let the plugin pick dynamically).
#[derive(Clone, Debug, Default)]
pub struct VisionConfig {
    pub vision_models: Vec<String>,
    pub vision_model: Option<String>,
}

const FILENAME: &str = "vision.json";

fn path(workspace: &Path) -> std::path::PathBuf {
    workspace.join(".catalyst-code").join(FILENAME)
}

impl VisionConfig {
    /// Load from <workspace>/.catalyst-code/vision.json; default if absent/unreadable.
    pub fn load(workspace: &Path) -> Self {
        let p = path(workspace);
        let Ok(content) = std::fs::read_to_string(&p) else {
            return Self::default();
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) else {
            return Self::default();
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(unique: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("umans_vision_test_{unique}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn load_missing_file_is_default() {
        let dir = tmp("missing");
        let cfg = VisionConfig::load(&dir);
        assert!(cfg.vision_models.is_empty());
        assert!(cfg.vision_model.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = tmp("roundtrip");
        let cfg = VisionConfig {
            vision_models: vec!["umans-kimi-k2.6".into(), "umans-coder".into()],
            vision_model: Some("umans-kimi-k2.6".into()),
        };
        cfg.save(&dir);
        let loaded = VisionConfig::load(&dir);
        assert_eq!(loaded.vision_models, cfg.vision_models);
        assert_eq!(loaded.vision_model, Some("umans-kimi-k2.6".to_string()));
        assert!(loaded.has_vision("umans-coder"));
        assert!(!loaded.has_vision("umans-glm-5.2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_vision_model_becomes_none() {
        let dir = tmp("empty_vm");
        std::fs::create_dir_all(dir.join(".catalyst-code")).unwrap();
        std::fs::write(
            dir.join(".catalyst-code").join("vision.json"),
            r#"{"vision_models":["x"],"vision_model":""}"#,
        )
        .unwrap();
        let cfg = VisionConfig::load(&dir);
        assert_eq!(cfg.vision_models, vec!["x".to_string()]);
        assert!(cfg.vision_model.is_none()); // empty string => None
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
        assert!(cfg.vision_models.is_empty());
        assert!(cfg.vision_model.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
