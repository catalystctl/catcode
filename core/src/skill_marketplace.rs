use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

const SEARCH_URL: &str = "https://skills.sh/api/search";
const DOWNLOAD_URL: &str = "https://skills.sh/api/download";
const MAX_FILES: usize = 1_000;
const MAX_BYTES: usize = 25 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillScope {
    Project,
    Global,
}

impl SkillScope {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "project" | "workspace" => Ok(Self::Project),
            "global" | "user" => Ok(Self::Global),
            _ => Err("skill scope must be 'project' or 'global'".into()),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Global => "global",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketplaceSkill {
    pub id: String,
    #[serde(rename = "skillId", default)]
    pub skill_id: String,
    pub name: String,
    #[serde(default)]
    pub installs: u64,
    pub source: String,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    skills: Vec<MarketplaceSkill>,
}

#[derive(Debug, Deserialize)]
struct DownloadResponse {
    files: Vec<SnapshotFile>,
    hash: String,
}

#[derive(Debug, Deserialize)]
struct SnapshotFile {
    path: String,
    contents: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LockEntry {
    pub source: String,
    #[serde(rename = "sourceType")]
    pub source_type: String,
    #[serde(rename = "skillPath", skip_serializing_if = "Option::is_none")]
    pub skill_path: Option<String>,
    #[serde(rename = "computedHash")]
    pub computed_hash: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LockFile {
    #[serde(default = "lock_version")]
    version: u32,
    #[serde(default)]
    skills: BTreeMap<String, LockEntry>,
}

fn lock_version() -> u32 {
    1
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct UserState {
    #[serde(default)]
    disclaimer_accepted: bool,
}

pub fn disclaimer_accepted() -> bool {
    state_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str::<UserState>(&raw).ok())
        .map(|state| state.disclaimer_accepted)
        .unwrap_or(false)
}

pub fn accept_disclaimer() -> Result<(), String> {
    let path = state_path().ok_or_else(|| "home directory is unavailable".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(&UserState {
        disclaimer_accepted: true,
    })
    .map_err(|e| format!("failed to serialize skill explorer state: {e}"))?;
    crate::fsutil::atomic_write_secure(&path, body.as_bytes())
        .map_err(|e| format!("failed to save skill explorer state: {e}"))
}

pub async fn search(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<MarketplaceSkill>, String> {
    require_disclaimer()?;
    let query = query.trim();
    if query.chars().count() < 2 {
        return Err("skill search requires at least 2 characters".into());
    }
    let response = client
        .get(SEARCH_URL)
        .query(&[("q", query), ("limit", "50")])
        .header("User-Agent", "catcode-skills")
        .send()
        .await
        .map_err(|e| format!("skills.sh search failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("skills.sh search returned HTTP {status}"));
    }
    let mut skills = response
        .json::<SearchResponse>()
        .await
        .map_err(|e| format!("invalid skills.sh search response: {e}"))?
        .skills;
    skills.retain(|skill| valid_source(&skill.source) && valid_name(&skill.name));
    skills.sort_by(|a, b| {
        b.installs
            .cmp(&a.installs)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(skills)
}

pub async fn install(
    client: &reqwest::Client,
    workspace: &Path,
    source: &str,
    name: &str,
    scope: SkillScope,
) -> Result<LockEntry, String> {
    require_disclaimer()?;
    validate_identity(source, name)?;
    let snapshot = download(client, source, name).await?;
    install_snapshot(workspace, source, name, scope, snapshot)
}

pub async fn update(
    client: &reqwest::Client,
    workspace: &Path,
    name: &str,
    scope: SkillScope,
) -> Result<(LockEntry, bool), String> {
    require_disclaimer()?;
    validate_name(name)?;
    let lock_path = lock_path(workspace, scope)?;
    let lock = read_lock(&lock_path);
    let existing = lock
        .skills
        .get(name)
        .cloned()
        .ok_or_else(|| format!("skill '{name}' is not managed in {} scope", scope.as_str()))?;
    let snapshot = download(client, &existing.source, name).await?;
    if snapshot.hash == existing.computed_hash {
        return Ok((existing, false));
    }
    let entry = install_snapshot(workspace, &existing.source, name, scope, snapshot)?;
    Ok((entry, true))
}

pub fn remove(workspace: &Path, name: &str, scope: SkillScope) -> Result<(), String> {
    require_disclaimer()?;
    validate_name(name)?;
    let lock_path = lock_path(workspace, scope)?;
    let mut lock = read_lock(&lock_path);
    if lock.skills.remove(name).is_none() {
        return Err(format!(
            "skill '{name}' is not managed in {} scope",
            scope.as_str()
        ));
    }
    let dir = skills_dir(workspace, scope)?.join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| format!("failed to remove {}: {e}", dir.display()))?;
    }
    write_lock(&lock_path, &lock)
}

pub fn installed(workspace: &Path) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for scope in [SkillScope::Project, SkillScope::Global] {
        let Ok(path) = lock_path(workspace, scope) else {
            continue;
        };
        for (name, entry) in read_lock(&path).skills {
            let location = skills_dir(workspace, scope)
                .ok()
                .map(|p| p.join(&name).display().to_string());
            out.push(serde_json::json!({
                "name": name,
                "source": entry.source,
                "scope": scope.as_str(),
                "hash": entry.computed_hash,
                "location": location,
            }));
        }
    }
    out.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["name"].as_str().unwrap_or_default())
    });
    out
}

fn install_snapshot(
    workspace: &Path,
    source: &str,
    name: &str,
    scope: SkillScope,
    snapshot: DownloadResponse,
) -> Result<LockEntry, String> {
    validate_snapshot(&snapshot)?;
    let base = skills_dir(workspace, scope)?;
    std::fs::create_dir_all(&base)
        .map_err(|e| format!("failed to create {}: {e}", base.display()))?;
    let unique = format!(".{}-install-{}", name, std::process::id());
    let stage = base.join(unique);
    if stage.exists() {
        std::fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&stage).map_err(|e| format!("failed to stage skill: {e}"))?;
    let write_result = (|| {
        for file in &snapshot.files {
            let rel = safe_relative_path(&file.path)?;
            let target = stage.join(rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create skill directory: {e}"))?;
            }
            std::fs::write(&target, file.contents.as_bytes())
                .map_err(|e| format!("failed to write {}: {e}", target.display()))?;
        }
        Ok::<(), String>(())
    })();
    if let Err(error) = write_result {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(error);
    }

    let dest = base.join(name);
    let backup = base.join(format!(".{}-backup-{}", name, std::process::id()));
    if backup.exists() {
        let _ = std::fs::remove_dir_all(&backup);
    }
    if dest.exists() {
        std::fs::rename(&dest, &backup)
            .map_err(|e| format!("failed to replace {}: {e}", dest.display()))?;
    }
    if let Err(error) = std::fs::rename(&stage, &dest) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, &dest);
        }
        return Err(format!("failed to install {}: {error}", dest.display()));
    }
    let entry = LockEntry {
        source: source.to_string(),
        source_type: "github".into(),
        skill_path: Some("SKILL.md".into()),
        computed_hash: snapshot.hash,
    };
    let lock_path = lock_path(workspace, scope)?;
    let mut lock = read_lock(&lock_path);
    lock.skills.insert(name.to_string(), entry.clone());
    if let Err(error) = write_lock(&lock_path, &lock) {
        let _ = std::fs::remove_dir_all(&dest);
        if backup.exists() {
            let _ = std::fs::rename(&backup, &dest);
        }
        return Err(error);
    }
    let _ = std::fs::remove_dir_all(&backup);
    Ok(entry)
}

async fn download(
    client: &reqwest::Client,
    source: &str,
    name: &str,
) -> Result<DownloadResponse, String> {
    let url = format!("{DOWNLOAD_URL}/{source}/{name}");
    let response = client
        .get(url)
        .header("User-Agent", "catcode-skills")
        .send()
        .await
        .map_err(|e| format!("skill download failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("skills.sh download returned HTTP {status}"));
    }
    response
        .json::<DownloadResponse>()
        .await
        .map_err(|e| format!("invalid skills.sh download response: {e}"))
}

fn validate_snapshot(snapshot: &DownloadResponse) -> Result<(), String> {
    if snapshot.files.is_empty() || snapshot.files.len() > MAX_FILES {
        return Err(format!(
            "skill snapshot has an invalid file count (max {MAX_FILES})"
        ));
    }
    let bytes: usize = snapshot.files.iter().map(|file| file.contents.len()).sum();
    if bytes > MAX_BYTES {
        return Err(format!(
            "skill snapshot exceeds {} MiB",
            MAX_BYTES / 1024 / 1024
        ));
    }
    let mut has_skill = false;
    for file in &snapshot.files {
        let path = safe_relative_path(&file.path)?;
        if path == Path::new("SKILL.md") {
            has_skill = true;
            if !file.contents.starts_with("---") {
                return Err("downloaded SKILL.md has no YAML frontmatter".into());
            }
        }
    }
    if !has_skill {
        return Err("skill snapshot does not contain a root SKILL.md".into());
    }
    if snapshot.hash.trim().is_empty() {
        return Err("skill snapshot has no content hash".into());
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if path.is_absolute() || value.contains('\\') {
        return Err(format!("unsafe skill path: {value}"));
    }
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!("unsafe skill path: {value}"));
    }
    Ok(path.to_path_buf())
}

fn validate_identity(source: &str, name: &str) -> Result<(), String> {
    if !valid_source(source) {
        return Err("skill source must be a GitHub owner/repository identifier".into());
    }
    validate_name(name)
}

fn validate_name(name: &str) -> Result<(), String> {
    if valid_name(name) {
        Ok(())
    } else {
        Err("skill name contains unsupported characters".into())
    }
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
}

fn valid_source(value: &str) -> bool {
    let parts: Vec<&str> = value.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 100
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        })
}

fn require_disclaimer() -> Result<(), String> {
    if disclaimer_accepted() {
        Ok(())
    } else {
        Err("skill explorer disclaimer must be accepted before browsing or managing marketplace skills".into())
    }
}

fn state_path() -> Option<PathBuf> {
    crate::config::home_dir().map(|home| home.join(".config/catalyst-code/skill-explorer.json"))
}

fn skills_dir(workspace: &Path, scope: SkillScope) -> Result<PathBuf, String> {
    match scope {
        SkillScope::Project => Ok(workspace.join(".catalyst-code/skills")),
        SkillScope::Global => crate::config::home_dir()
            .map(|home| home.join(".catalyst-code/skills"))
            .ok_or_else(|| "home directory is unavailable".into()),
    }
}

fn lock_path(workspace: &Path, scope: SkillScope) -> Result<PathBuf, String> {
    match scope {
        SkillScope::Project => Ok(workspace.join("skills-lock.json")),
        SkillScope::Global => crate::config::home_dir()
            .map(|home| home.join(".catalyst-code/skills-lock.json"))
            .ok_or_else(|| "home directory is unavailable".into()),
    }
}

fn read_lock(path: &Path) -> LockFile {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|body| serde_json::from_str(&body).ok())
        .unwrap_or_else(|| LockFile {
            version: lock_version(),
            skills: BTreeMap::new(),
        })
}

fn write_lock(path: &Path, lock: &LockFile) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(lock)
        .map_err(|e| format!("failed to serialize skill lock: {e}"))?
        + "\n";
    crate::fsutil::atomic_write_str(path, &body)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_snapshot_paths() {
        assert!(safe_relative_path("../SKILL.md").is_err());
        assert!(safe_relative_path("/tmp/SKILL.md").is_err());
        assert!(safe_relative_path("rules/good.md").is_ok());
    }

    #[test]
    fn validates_marketplace_identity() {
        assert!(validate_identity("anthropics/skills", "frontend-design").is_ok());
        assert!(validate_identity("https://evil.test/x", "frontend-design").is_err());
        assert!(validate_identity("anthropics/skills", "../escape").is_err());
    }

    #[test]
    fn snapshot_requires_root_skill_file() {
        let missing = DownloadResponse {
            files: vec![SnapshotFile {
                path: "README.md".into(),
                contents: "x".into(),
            }],
            hash: "abc".into(),
        };
        assert!(validate_snapshot(&missing).is_err());
    }
}
