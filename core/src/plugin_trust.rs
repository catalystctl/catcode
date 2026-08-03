// Plugin trust store: per-project trust/deny decisions for project-scoped
// plugins (`.catalyst-code/plugins/*` shipped by a repo).
//
// Security model: a repo you `cd` into must not auto-run hook scripts with
// your privileges, so project plugins load only when the USER opts in. The
// trust prompt (TUI modal, `/plugin-trust`) records a decision per plugin,
// and this store persists those decisions in the USER-owned config dir
// (`~/.config/catalyst-code/plugin-trust.json`), keyed by canonical project
// root — never inside the repo, so a repository cannot self-trust its own
// hooks by committing a file.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// User-owned trust store: canonical project root → plugin directory key →
/// `"trust"` | `"deny"`, plus explicit user-installed workspace plugins.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PluginTrustStore {
    pub projects: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    pub installed: HashMap<String, Vec<String>>,
}

/// Absolute path of the trust store in the user config dir.
pub fn plugin_trust_store_path() -> Option<std::path::PathBuf> {
    crate::config::home_dir().map(|h| h.join(".config/catalyst-code/plugin-trust.json"))
}

impl PluginTrustStore {
    /// Load from `path`. A missing or unreadable file yields an empty store
    /// (best-effort; trust decisions are advisory, never blocking).
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Decisions recorded for `project` (canonical root): name → trust|deny.
    pub fn decisions_for(&self, project: &Path) -> HashMap<String, String> {
        self.projects
            .get(&project_key(project))
            .cloned()
            .unwrap_or_default()
    }

    /// Record decisions for `project`, replacing its previous set.
    pub fn set_decisions(&mut self, project: &Path, decisions: &HashMap<String, String>) {
        self.projects
            .insert(project_key(project), decisions.clone());
    }

    /// Return the user-approved workspace plugin directory keys.
    pub fn installed_plugins_for(&self, project: &Path) -> std::collections::HashSet<String> {
        self.installed
            .get(&project_key(project))
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    /// Replace the user-approved workspace plugin directory keys.
    pub fn set_installed_plugins(
        &mut self,
        project: &Path,
        plugins: &std::collections::HashSet<String>,
    ) {
        let mut values: Vec<String> = plugins.iter().cloned().collect();
        values.sort();
        self.installed.insert(project_key(project), values);
    }

    /// Persist to `path` atomically. Creates the parent dir as needed.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("plugin trust store serialization failed: {e}"))?;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        crate::fsutil::atomic_write_str(path, &json)
            .map_err(|e| format!("failed to write plugin trust store {}: {e}", path.display()))
    }
}

fn project_key(project: &Path) -> String {
    std::fs::canonicalize(project)
        .unwrap_or_else(|_| project.to_path_buf())
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_decisions() {
        let tmp = std::env::temp_dir().join(format!("plugin-trust-test-{}", std::process::id()));
        let path = tmp.join("store.json");
        let _ = std::fs::remove_file(&path);

        let mut store = PluginTrustStore::default();
        let mut decisions = HashMap::new();
        decisions.insert("shady".into(), "deny".into());
        decisions.insert("linter".into(), "trust".into());
        store.set_decisions(Path::new("/ws/one"), &decisions);
        store.save(&path).unwrap();

        let loaded = PluginTrustStore::load(&path);
        let got = loaded.decisions_for(Path::new("/ws/one"));
        assert_eq!(got.get("shady").map(String::as_str), Some("deny"));
        assert_eq!(got.get("linter").map(String::as_str), Some("trust"));
        // A different project has no recorded decisions.
        assert!(loaded.decisions_for(Path::new("/ws/two")).is_empty());
        let mut installed = std::collections::HashSet::new();
        installed.insert("/ws/one/.catalyst-code/plugins/linter".into());
        store.set_installed_plugins(Path::new("/ws/one"), &installed);
        store.save(&path).unwrap();
        assert!(PluginTrustStore::load(&path)
            .installed_plugins_for(Path::new("/ws/one"))
            .contains("/ws/one/.catalyst-code/plugins/linter"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_loads_empty() {
        let store = PluginTrustStore::load(Path::new("/nonexistent/trust.json"));
        assert!(store.projects.is_empty());
    }
}
