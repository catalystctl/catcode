// persistent memory system. Stores named memories as markdown files with
// YAML-like frontmatter under ~/.config/umans-harness/memory/<project-hash>/.
// Memories are scoped per workspace (hashed canonical path) and injected into
// the system prompt when relevant keyword matches are found in the user's prompt.
// ponytail: no DB, no extra crate — just markdown files on disk.
//
// Only memory_injection is wired (main.rs). The save/scan/hash half (Store::save,
// rebuild_index, slugify, scan_memories, save_memory, project_hash) is a staged
// feature not yet bound to a Command; keep it + its tests but silence dead-code
// until it's wired so clippy stays clean.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct MemoryEntry {
    pub name: String,
    pub description: String,
    pub mem_type: String,
    pub content: String,
    pub path: PathBuf,
}

// ---- hash ----

/// Hash the workspace path for scoped storage. Deterministic, using FNV-1a
/// on the canonicalized absolute path of `cwd`. Returns 16 hex chars.
pub fn project_hash(cwd: &str) -> String {
    let p = PathBuf::from(cwd);
    let canonical = std::fs::canonicalize(&p).unwrap_or(p);
    let h = fnv1a(canonical.to_string_lossy().as_bytes());
    format!("{:016x}", h)
}

fn hash_workspace(workspace: &Path) -> String {
    let canonical = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    let h = fnv1a(canonical.to_string_lossy().as_bytes());
    format!("{:016x}", h)
}

fn fnv1a(s: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in s {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// ---- store (scoped to a root) ----

struct Store {
    root: PathBuf,
}

impl Store {
    fn default_root() -> PathBuf {
        let home = crate::config::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".config/umans-harness/memory")
    }

    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn dir(&self, workspace: &Path) -> PathBuf {
        self.root.join(hash_workspace(workspace))
    }

    fn scan(&self, workspace: &Path) -> Vec<MemoryEntry> {
        scan_dir(&self.dir(workspace))
    }

    fn save(
        &self,
        workspace: &Path,
        name: &str,
        content: &str,
        mem_type: &str,
        description: &str,
    ) -> Result<PathBuf, String> {
        let dir = self.dir(workspace);
        std::fs::create_dir_all(&dir).map_err(|e| format!("failed to create memory dir: {e}"))?;

        let slug = slugify(name);
        let filename = format!("{}.md", slug);
        let path = dir.join(&filename);

        let body = format!(
            "---\nname: {}\ndescription: {}\ntype: {}\n---\n{}",
            name, description, mem_type, content
        );

        std::fs::write(&path, &body)
            .map_err(|e| format!("failed to write memory file {filename:?}: {e}"))?;

        rebuild_index(&dir)?;

        Ok(path)
    }
}

// ---- public API ----

/// Scan all memory files for a workspace, returning parsed entries.
/// Skips the index file (MEMORY.md) and any unparseable files.
pub fn scan_memories(workspace: &Path) -> Vec<MemoryEntry> {
    Store::new(Store::default_root()).scan(workspace)
}

/// Write a memory file (with frontmatter) and rebuild the MEMORY.md index.
/// The filename is derived from `name` (slugified). Existing files are
/// overwritten silently.
pub fn save_memory(
    workspace: &Path,
    name: &str,
    content: &str,
    mem_type: &str,
    description: &str,
) -> Result<PathBuf, String> {
    Store::new(Store::default_root()).save(workspace, name, content, mem_type, description)
}

/// Build a string to inject into the system prompt with memories relevant to
/// the user's current prompt. Returns an empty string if no memories match.
pub fn memory_injection(workspace: &Path, prompt: &str) -> String {
    let store = Store::new(Store::default_root());
    let memories = store.scan(workspace);
    build_injection(&memories, prompt)
}

fn build_injection(memories: &[MemoryEntry], prompt: &str) -> String {
    if memories.is_empty() {
        return String::new();
    }
    let relevant: Vec<&MemoryEntry> = memories.iter().filter(|m| is_relevant(m, prompt)).collect();
    if relevant.is_empty() {
        return String::new();
    }
    let mut out = String::from("[PERSISTENT MEMORIES]\n");
    for m in &relevant {
        let desc_part = if m.description.is_empty() {
            String::new()
        } else {
            format!(": {}", m.description)
        };
        out.push_str(&format!("- **{}** ({}){}\n", m.name, m.mem_type, desc_part));
        if !m.content.is_empty() {
            let preview: String = m.content.lines().take(5).collect::<Vec<_>>().join("\n");
            out.push_str(&format!("  {}\n", preview));
        }
    }
    out
}

// ---- scan internals ----

fn scan_dir(dir: &Path) -> Vec<MemoryEntry> {
    let mut entries = Vec::new();
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return entries,
    };
    for e in rd.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("md") {
            continue;
        }
        if path.file_name().and_then(|x| x.to_str()) == Some("MEMORY.md") {
            continue;
        }
        if let Some(entry) = parse_memory_file(&path) {
            entries.push(entry);
        }
    }
    entries
}

// ---- frontmatter parser ----

/// Parse a memory markdown file. Returns None if the file can't be read or
/// has no valid frontmatter block (--- ... ---). Frontmatter fields are
/// simple `key: value` lines (YAML-like, hand-rolled). Everything after the
/// closing `---` is the content.
fn parse_memory_file(path: &Path) -> Option<MemoryEntry> {
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---\n") && !trimmed.starts_with("---\r\n") {
        return None;
    }
    let after_open = &trimmed[3..];
    let after_open = after_open
        .strip_prefix('\n')
        .or_else(|| after_open.strip_prefix("\r\n"))
        .unwrap_or(after_open);
    let end_pos = find_frontmatter_end(after_open)?;
    let fm_block = &after_open[..end_pos];
    let body_start = end_pos + 3;
    let rest = &after_open[body_start..];
    let content = rest
        .strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"))
        .unwrap_or(rest)
        .to_string();

    let mut name = String::new();
    let mut description = String::new();
    let mut mem_type = String::new();

    for line in fm_block.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (key, val) = match line.split_once(':') {
            Some((k, v)) => (k.trim().to_lowercase(), v.trim().to_string()),
            None => continue,
        };
        match key.as_str() {
            "name" => name = val,
            "description" => description = val,
            "type" => mem_type = val,
            _ => {}
        }
    }

    if name.is_empty() {
        return None;
    }

    Some(MemoryEntry {
        name,
        description,
        mem_type,
        content,
        path: path.to_path_buf(),
    })
}

/// Find the closing `---` line in a frontmatter block. Returns the byte offset
/// of the `---` line within `s`.
fn find_frontmatter_end(s: &str) -> Option<usize> {
    let mut offset = 0usize;
    for line in s.lines() {
        if line == "---" {
            return Some(offset);
        }
        offset += line.len() + 1;
    }
    None
}

// ---- index ----

/// Rebuild MEMORY.md from all .md files in the memory directory.
fn rebuild_index(dir: &Path) -> Result<(), String> {
    let entries = scan_dir(dir);
    let mut idx = String::from("# Memory Index\n\n");
    if entries.is_empty() {
        idx.push_str("_(no memories yet)_\n");
    } else {
        for e in &entries {
            let slug = slugify(&e.name);
            idx.push_str(&format!(
                "- [{}](./{}.md) — {}\n",
                e.name, slug, e.description
            ));
        }
    }
    let idx_path = dir.join("MEMORY.md");
    std::fs::write(&idx_path, &idx).map_err(|e| format!("failed to write MEMORY.md: {e}"))
}

// ---- relevance ----

/// Basic keyword matching: if any significant word (>2 chars, not a stop-word)
/// from the memory's name or description appears in the prompt (case-insensitive),
/// the memory is considered relevant.
fn is_relevant(entry: &MemoryEntry, prompt: &str) -> bool {
    let prompt_lower = prompt.to_lowercase();
    let text = format!("{} {}", entry.name, entry.description);
    let keywords: Vec<&str> = text
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .filter(|w| !is_stopword(w))
        .collect();
    keywords
        .iter()
        .any(|kw| prompt_lower.contains(&kw.to_lowercase()))
}

fn is_stopword(w: &str) -> bool {
    matches!(
        w.to_lowercase().as_str(),
        "the"
            | "and"
            | "for"
            | "with"
            | "that"
            | "this"
            | "from"
            | "are"
            | "was"
            | "has"
            | "not"
            | "but"
            | "its"
            | "can"
    )
}

// ---- helpers ----

fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            out.push(c.to_ascii_lowercase());
        } else {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

// ---- tests ----

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("umans_harness_memory_test_{n}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn fake_workspace(name: &str) -> PathBuf {
        let ws = std::env::temp_dir().join(format!("umans_harness_memory_ws_{name}"));
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join(".gitkeep"), "").ok();
        ws
    }

    fn test_store(root: &Path) -> Store {
        Store::new(root.to_path_buf())
    }

    #[test]
    fn project_hash_is_deterministic() {
        let a = project_hash("/tmp/foo");
        let b = project_hash("/tmp/foo");
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn project_hash_different_for_different_paths() {
        let a = project_hash("/tmp/aaa");
        let b = project_hash("/tmp/bbb");
        assert_ne!(a, b);
    }

    #[test]
    fn scan_returns_empty_for_nonexistent_dir() {
        let root = tmp_root();
        let ws = fake_workspace("nonexistent");
        let store = test_store(&root);
        assert!(store.scan(&ws).is_empty());
    }

    #[test]
    fn save_and_scan_roundtrip() {
        let root = tmp_root();
        let ws = fake_workspace("roundtrip");
        let store = test_store(&root);

        let p1 = store
            .save(
                &ws,
                "user preferences",
                "Always use tabs",
                "user",
                "prefers tabs over spaces",
            )
            .unwrap();
        assert!(p1.exists());
        let p2 = store
            .save(
                &ws,
                "Project Rules",
                "No panics in production",
                "project",
                "code rules",
            )
            .unwrap();
        assert!(p2.exists());

        let entries = store.scan(&ws);
        assert_eq!(entries.len(), 2);

        let user = entries
            .iter()
            .find(|e| e.name == "user preferences")
            .unwrap();
        assert_eq!(user.mem_type, "user");
        assert_eq!(user.content, "Always use tabs");
        assert_eq!(user.description, "prefers tabs over spaces");

        let proj = entries.iter().find(|e| e.name == "Project Rules").unwrap();
        assert_eq!(proj.mem_type, "project");
        assert_eq!(proj.content, "No panics in production");
    }

    #[test]
    fn index_file_created_after_save() {
        let root = tmp_root();
        let ws = fake_workspace("idx");
        let store = test_store(&root);

        store.save(&ws, "test", "body", "user", "desc").unwrap();
        let dir = store.dir(&ws);
        let idx_path = dir.join("MEMORY.md");
        assert!(idx_path.exists());

        let idx_content = std::fs::read_to_string(&idx_path).unwrap();
        assert!(idx_content.contains("test"));
        assert!(idx_content.contains("./test.md"));
    }

    #[test]
    fn index_not_counted_as_memory() {
        let root = tmp_root();
        let ws = fake_workspace("skipidx");
        let store = test_store(&root);

        store
            .save(&ws, "rule", "cpp", "project", "c++ rules")
            .unwrap();
        let dir = store.dir(&ws);
        std::fs::write(dir.join("garbage.md"), "no frontmatter here").unwrap();

        let entries = store.scan(&ws);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "rule");
    }

    #[test]
    fn relevance_matches_keywords() {
        let e = MemoryEntry {
            name: "TypeScript style".into(),
            description: "prefer strict mode, no enums".into(),
            mem_type: "user".into(),
            content: String::new(),
            path: PathBuf::from("/fake/ts.md"),
        };
        assert!(is_relevant(&e, "write a strict TypeScript component"));
        assert!(is_relevant(&e, "use enums in this file"));
        assert!(is_relevant(&e, "TypeScript rules"));
        assert!(!is_relevant(&e, "write a Python script"));
        assert!(!is_relevant(&e, "hello world"));
    }

    #[test]
    fn relevance_skips_stopwords() {
        let e = MemoryEntry {
            name: "formatting".into(),
            description: "the standard is to use 2-space indent and it has trailing commas".into(),
            mem_type: "project".into(),
            content: String::new(),
            path: PathBuf::from("/fake/fmt.md"),
        };
        assert!(!is_relevant(&e, "the quick brown fox"));
        assert!(!is_relevant(&e, "this and that"));
        assert!(is_relevant(&e, "use standard formatting please"));
        assert!(is_relevant(&e, "adjust indent width"));
    }

    #[test]
    fn memory_injection_builds_string() {
        let root = tmp_root();
        let ws = fake_workspace("inj");
        let store = test_store(&root);

        store
            .save(
                &ws,
                "test rules",
                "run tests with jest",
                "project",
                "Jest is the test framework",
            )
            .unwrap();
        store
            .save(&ws, "indent", "always use tabs", "user", "tab width 4")
            .unwrap();

        let memories = store.scan(&ws);
        let injection = build_injection(&memories, "please add jest tests for the component");
        assert!(injection.contains("[PERSISTENT MEMORIES]"));
        assert!(injection.contains("test rules"));
        assert!(injection.contains("Jest is the test framework"));
        assert!(!injection.contains("indent"));
    }

    #[test]
    fn memory_injection_empty_for_no_relevant() {
        let root = tmp_root();
        let ws = fake_workspace("noinj");
        let store = test_store(&root);

        store
            .save(&ws, "rust rules", "no unsafe", "project", "safe Rust only")
            .unwrap();
        let memories = store.scan(&ws);
        let injection = build_injection(&memories, "write a python script");
        assert!(injection.is_empty());
    }

    #[test]
    fn save_overwrites_existing() {
        let root = tmp_root();
        let ws = fake_workspace("over");
        let store = test_store(&root);

        store
            .save(&ws, "my rule", "first version", "user", "desc 1")
            .unwrap();
        store
            .save(&ws, "my rule", "second version", "user", "desc 2")
            .unwrap();

        let entries = store.scan(&ws);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "second version");
        assert_eq!(entries[0].description, "desc 2");
    }

    #[test]
    fn slugify_produces_safe_filenames() {
        assert_eq!(slugify("user preferences"), "user-preferences");
        assert_eq!(slugify("TypeScript Rules!"), "typescript-rules");
        assert_eq!(slugify("  spaces  "), "spaces");
        assert_eq!(slugify("a/b:c"), "a-b-c");
    }

    #[test]
    fn frontmatter_parses_correctly() {
        let root = tmp_root();
        let ws = fake_workspace("fm");
        let store = test_store(&root);
        let dir = store.dir(&ws);
        std::fs::create_dir_all(&dir).unwrap();

        let md = "---\nname: test config\ndescription: some desc\ntype: user\n---\nHere is the body.\nMultiline.\n";
        std::fs::write(dir.join("test.md"), md).unwrap();

        let entries = store.scan(&ws);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "test config");
        assert_eq!(entries[0].description, "some desc");
        assert_eq!(entries[0].mem_type, "user");
        assert_eq!(entries[0].content, "Here is the body.\nMultiline.\n");
    }

    #[test]
    fn frontmatter_rejects_missing_name() {
        let root = tmp_root();
        let ws = fake_workspace("noname");
        let store = test_store(&root);
        let dir = store.dir(&ws);
        std::fs::create_dir_all(&dir).unwrap();

        let md = "---\ndescription: no name here\ntype: user\n---\nbody\n";
        std::fs::write(dir.join("unnamed.md"), md).unwrap();

        let entries = store.scan(&ws);
        assert!(entries.is_empty());
    }
}
