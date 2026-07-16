//! Living codebase index (spec §8) — offline, incremental, no external parsers.
//!
//! Stores compact JSONL under `learning/projects/<id>/index/`:
//! - `files.jsonl` — file metadata
//! - `symbols.jsonl` — declarations (lightweight regex/line scanners)
//! - `relations.jsonl` — imports / references (heuristic)
//! - `manifest.json` — content hashes + scan watermark
//!
//! Fail-open: index errors never abort a coding turn.

#![allow(dead_code)]
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::learning_store::{self, ProjectLearningPaths};
use crate::project_identity::ProjectIdentity;

const MANIFEST_SCHEMA: u32 = 1;
const MAX_FILE_BYTES: u64 = 1_500_000;
const MAX_SYMBOLS_PER_FILE: usize = 200;

/// Indexed file metadata (spec §8.3).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FileRecord {
    pub path: String,
    pub language: String,
    pub size: u64,
    pub content_hash: String,
    pub modified_at: u64,
    pub ignored: bool,
    pub generated: bool,
    pub binary: bool,
}

/// Indexed symbol declaration.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SymbolRecord {
    pub id: String,
    pub path: String,
    pub name: String,
    pub kind: String,
    pub language: String,
    pub line_start: u32,
    pub line_end: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_symbol: Option<String>,
}

/// Config / env / route key (no secret values).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigKeyRecord {
    pub path: String,
    pub key: String,
    pub kind: String,
}

/// Test name discovered in a source file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestRefRecord {
    pub path: String,
    pub name: String,
    pub language: String,
}

/// Indexed relationship edge.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RelationRecord {
    pub from_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_symbol: Option<String>,
    pub to_name: String,
    pub kind: String,
    pub confidence: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct IndexManifest {
    schema_version: u32,
    project_id: String,
    #[serde(default)]
    file_hashes: HashMap<String, String>,
    last_full_scan_at: u64,
    last_incremental_at: u64,
    file_count: usize,
    symbol_count: usize,
}

/// Directories always skipped (spec §8.5).
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".catalyst-code",
    "vendor",
    ".next",
    "coverage",
];

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn detect_language(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "rs" => "rust",
        "go" => "go",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "py" => "python",
        "md" | "mdx" => "markdown",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        _ => "unknown",
    }
}

fn is_probably_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|&b| b == 0)
}

fn is_generated_path(rel: &str) -> bool {
    let lower = rel.to_lowercase();
    lower.contains("generated")
        || lower.ends_with(".min.js")
        || lower.ends_with(".min.css")
        || lower.contains("/gen/")
}

fn should_skip_dir(name: &str) -> bool {
    SKIP_DIRS.iter().any(|d| *d == name)
}

/// Walk workspace collecting relative paths, honoring skip dirs + `.gitignore`
/// basics (best-effort: only top-level patterns without negation).
fn collect_source_files(workspace: &Path) -> Vec<PathBuf> {
    let gitignore = load_simple_gitignore(workspace);
    let mut out = Vec::new();
    let mut stack = vec![workspace.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for ent in entries.flatten() {
            let path = ent.path();
            let name = ent.file_name().to_string_lossy().to_string();
            if name.starts_with('.') && name != ".github" {
                // Allow scanning dotfiles selectively; skip most hidden dirs.
                if path.is_dir() && name != ".github" {
                    continue;
                }
            }
            if path.is_dir() {
                if should_skip_dir(&name) {
                    continue;
                }
                stack.push(path);
            } else if path.is_file() {
                let rel = path
                    .strip_prefix(workspace)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                if gitignore_matches(&gitignore, &rel) {
                    continue;
                }
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn load_simple_gitignore(workspace: &Path) -> Vec<String> {
    let p = workspace.join(".gitignore");
    match std::fs::read_to_string(p) {
        Ok(s) => s
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('!'))
            .map(|l| l.trim_start_matches('/').to_string())
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn gitignore_matches(patterns: &[String], rel: &str) -> bool {
    for pat in patterns {
        if pat.ends_with('/') {
            let dir = pat.trim_end_matches('/');
            if rel == dir || rel.starts_with(&format!("{dir}/")) {
                return true;
            }
        } else if pat.contains('*') {
            // Very small glob: only trailing *
            if let Some(prefix) = pat.strip_suffix('*') {
                if rel.starts_with(prefix) || rel.split('/').last().unwrap_or("").starts_with(prefix)
                {
                    return true;
                }
            }
        } else if rel == pat || rel.ends_with(&format!("/{pat}")) || rel.starts_with(&format!("{pat}/"))
        {
            return true;
        }
    }
    false
}

/// Extract symbols + import relations for a file's text.
fn extract_symbols_and_relations(
    rel: &str,
    language: &str,
    text: &str,
) -> (Vec<SymbolRecord>, Vec<RelationRecord>) {
    let mut symbols = Vec::new();
    let mut relations = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
            continue;
        }
        if let Some((kind, name, vis, sig)) = match language {
            "rust" => parse_rust_decl(trimmed),
            "go" => parse_go_decl(trimmed),
            "typescript" | "javascript" => parse_ts_decl(trimmed),
            "python" => parse_python_decl(trimmed),
            _ => parse_generic_decl(trimmed),
        } {
            if symbols.len() < MAX_SYMBOLS_PER_FILE {
                symbols.push(SymbolRecord {
                    id: format!("{rel}::{name}"),
                    path: rel.to_string(),
                    name: name.clone(),
                    kind: kind.to_string(),
                    language: language.to_string(),
                    line_start: line_no,
                    line_end: line_no,
                    signature: sig,
                    visibility: vis,
                    parent_symbol: None,
                });
            }
        }
        if let Some(imported) = parse_import(language, trimmed) {
            relations.push(RelationRecord {
                from_path: rel.to_string(),
                from_symbol: None,
                to_name: imported,
                kind: "imports".into(),
                confidence: 0.85,
            });
        }
    }
    (symbols, relations)
}

fn parse_rust_decl(line: &str) -> Option<(&'static str, String, Option<String>, Option<String>)> {
    let vis = if line.starts_with("pub ") || line.starts_with("pub(") {
        Some("pub".into())
    } else {
        None
    };
    let l = line.trim_start_matches("pub(crate) ").trim_start_matches("pub ");
    let l = l.trim_start_matches("async ");
    if let Some(rest) = l.strip_prefix("fn ") {
        let name = rest.split('(').next()?.split('<').next()?.trim();
        if is_ident(name) {
            return Some(("function", name.into(), vis, Some(truncate(line, 120).into())));
        }
    }
    if let Some(rest) = l.strip_prefix("struct ") {
        let name = rest.split(|c: char| c == ' ' || c == '<' || c == '{').next()?.trim();
        if is_ident(name) {
            return Some(("struct", name.into(), vis, None));
        }
    }
    if let Some(rest) = l.strip_prefix("enum ") {
        let name = rest.split(|c: char| c == ' ' || c == '<' || c == '{').next()?.trim();
        if is_ident(name) {
            return Some(("enum", name.into(), vis, None));
        }
    }
    if let Some(rest) = l.strip_prefix("trait ") {
        let name = rest.split(|c: char| c == ' ' || c == '<' || c == '{').next()?.trim();
        if is_ident(name) {
            return Some(("trait", name.into(), vis, None));
        }
    }
    if let Some(rest) = l.strip_prefix("type ") {
        let name = rest.split('=').next()?.split('<').next()?.trim();
        if is_ident(name) {
            return Some(("type", name.into(), vis, None));
        }
    }
    if let Some(rest) = l.strip_prefix("mod ") {
        let name = rest.trim().trim_end_matches(';').trim().trim_end_matches('{').trim();
        if is_ident(name) {
            return Some(("module", name.into(), vis, None));
        }
    }
    None
}

fn parse_go_decl(line: &str) -> Option<(&'static str, String, Option<String>, Option<String>)> {
    if let Some(rest) = line.strip_prefix("func ") {
        let rest = rest.trim_start_matches('(');
        // method: func (t *T) Name(
        let name = if rest.contains(") ") {
            rest.split(") ").nth(1)?.split('(').next()?.trim()
        } else {
            rest.split('(').next()?.trim()
        };
        if is_ident(name) {
            let vis = if name.chars().next()?.is_uppercase() {
                Some("exported".into())
            } else {
                None
            };
            return Some(("function", name.into(), vis, Some(truncate(line, 120).into())));
        }
    }
    if let Some(rest) = line.strip_prefix("type ") {
        let name = rest.split_whitespace().next()?.trim();
        if is_ident(name) {
            return Some(("type", name.into(), None, None));
        }
    }
    None
}

fn parse_ts_decl(line: &str) -> Option<(&'static str, String, Option<String>, Option<String>)> {
    let l = line.trim_start_matches("export ").trim_start_matches("declare ");
    if let Some(rest) = l.strip_prefix("function ") {
        let name = rest.split('(').next()?.split('<').next()?.trim();
        if is_ident(name) {
            return Some(("function", name.into(), None, Some(truncate(line, 120).into())));
        }
    }
    if let Some(rest) = l.strip_prefix("class ") {
        let name = rest.split(|c: char| c == ' ' || c == '{' || c == '<').next()?.trim();
        if is_ident(name) {
            return Some(("class", name.into(), None, None));
        }
    }
    if let Some(rest) = l.strip_prefix("interface ") {
        let name = rest.split(|c: char| c == ' ' || c == '{' || c == '<').next()?.trim();
        if is_ident(name) {
            return Some(("interface", name.into(), None, None));
        }
    }
    if let Some(rest) = l.strip_prefix("type ") {
        let name = rest.split('=').next()?.split('<').next()?.trim();
        if is_ident(name) {
            return Some(("type", name.into(), None, None));
        }
    }
    None
}

fn parse_python_decl(line: &str) -> Option<(&'static str, String, Option<String>, Option<String>)> {
    if let Some(rest) = line.strip_prefix("def ") {
        let name = rest.split('(').next()?.trim();
        if is_ident(name) {
            return Some(("function", name.into(), None, Some(truncate(line, 120).into())));
        }
    }
    if let Some(rest) = line.strip_prefix("class ") {
        let name = rest.split(|c: char| c == '(' || c == ':' || c == ' ').next()?.trim();
        if is_ident(name) {
            return Some(("class", name.into(), None, None));
        }
    }
    None
}

fn parse_generic_decl(line: &str) -> Option<(&'static str, String, Option<String>, Option<String>)> {
    // Heading-like or identifier = patterns.
    if let Some(rest) = line.strip_prefix("# ") {
        let name = rest.trim();
        if !name.is_empty() {
            return Some(("heading", truncate(name, 64).into(), None, None));
        }
    }
    None
}

fn parse_import(language: &str, line: &str) -> Option<String> {
    match language {
        "rust" => {
            let l = line.trim_start_matches("pub ");
            if let Some(rest) = l.strip_prefix("use ") {
                let path = rest.trim().trim_end_matches(';').trim();
                return Some(truncate(path, 120).into());
            }
        }
        "go" => {
            if line.starts_with("import ") {
                let rest = line.trim_start_matches("import ").trim().trim_matches('"');
                if !rest.is_empty() && rest != "(" {
                    return Some(rest.into());
                }
            }
            if line.starts_with('"') && line.ends_with('"') {
                return Some(line.trim_matches('"').into());
            }
        }
        "typescript" | "javascript" => {
            if let Some(idx) = line.find(" from ") {
                let modname = line[idx + 6..].trim().trim_matches(';').trim_matches('"').trim_matches('\'');
                if !modname.is_empty() {
                    return Some(modname.into());
                }
            }
            if let Some(rest) = line.strip_prefix("import ") {
                let modname = rest.trim().trim_matches(';').trim_matches('"').trim_matches('\'');
                if !modname.is_empty() && !modname.starts_with('{') {
                    return Some(modname.into());
                }
            }
        }
        "python" => {
            if let Some(rest) = line.strip_prefix("import ") {
                return Some(rest.split(" as ").next()?.trim().into());
            }
            if let Some(rest) = line.strip_prefix("from ") {
                let modname = rest.split(" import ").next()?.trim();
                return Some(modname.into());
            }
        }
        _ => {}
    }
    None
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn truncate(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

fn index_one_file(workspace: &Path, abs: &Path) -> Option<(FileRecord, Vec<SymbolRecord>, Vec<RelationRecord>)> {
    let rel = abs
        .strip_prefix(workspace)
        .unwrap_or(abs)
        .to_string_lossy()
        .replace('\\', "/");
    let meta = std::fs::metadata(abs).ok()?;
    let size = meta.len();
    let modified_at = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let language = detect_language(abs).to_string();
    if size > MAX_FILE_BYTES {
        return Some((
            FileRecord {
                path: rel.clone(),
                language,
                size,
                content_hash: "too-large".into(),
                modified_at,
                ignored: false,
                generated: is_generated_path(&rel),
                binary: true,
            },
            Vec::new(),
            Vec::new(),
        ));
    }
    let bytes = std::fs::read(abs).ok()?;
    let binary = is_probably_binary(&bytes);
    let content_hash = fnv1a_hex(&bytes);
    if binary {
        return Some((
            FileRecord {
                path: rel.clone(),
                language,
                size,
                content_hash,
                modified_at,
                ignored: false,
                generated: is_generated_path(&rel),
                binary: true,
            },
            Vec::new(),
            Vec::new(),
        ));
    }
    // Never index secret-looking env assignment values — strip `KEY=value` bodies
    // from config-ish files by only keeping keys for env-like lines later.
    let text = String::from_utf8_lossy(&bytes);
    let (symbols, relations) = extract_symbols_and_relations(&rel, &language, &text);
    Some((
        FileRecord {
            path: rel.clone(),
            language,
            size,
            content_hash,
            modified_at,
            ignored: false,
            generated: is_generated_path(&rel),
            binary: false,
        },
        symbols,
        relations,
    ))
}


fn extract_config_tests_routes(
    workspace: &Path,
    files: &[FileRecord],
) -> (Vec<ConfigKeyRecord>, Vec<TestRefRecord>, Vec<ConfigKeyRecord>) {
    let mut configs = Vec::new();
    let mut tests = Vec::new();
    let mut routes = Vec::new();
    for f in files {
        if f.binary || f.ignored {
            continue;
        }
        let abs = workspace.join(&f.path);
        let Ok(text) = std::fs::read_to_string(&abs) else {
            continue;
        };
        let sample = if text.len() > 64_000 {
            &text[..64_000]
        } else {
            text.as_str()
        };
        let lower_path = f.path.to_lowercase();
        let is_test_file = lower_path.contains("test")
            || lower_path.ends_with("_test.go")
            || lower_path.ends_with(".test.ts")
            || lower_path.ends_with(".test.tsx");
        for line in sample.lines().take(500) {
            let trimmed = line.trim();
            // env::var("NAME")
            if let Some(idx) = trimmed.find("env::var(\"") {
                let rest = &trimmed[idx + 9..];
                if let Some(end) = rest.find('\"') {
                    let key = &rest[..end];
                    if !key.is_empty() && key.len() < 80 {
                        configs.push(ConfigKeyRecord {
                            path: f.path.clone(),
                            key: key.to_string(),
                            kind: "env".into(),
                        });
                    }
                }
            }
            if matches!(f.language.as_str(), "toml" | "json" | "yaml") {
                if let Some((k, _)) = trimmed.split_once('=') {
                    let k = k.trim().trim_matches('\"');
                    if !k.is_empty()
                        && k.len() < 80
                        && !k.starts_with('#')
                        && !k.starts_with('[')
                    {
                        configs.push(ConfigKeyRecord {
                            path: f.path.clone(),
                            key: k.to_string(),
                            kind: f.language.clone(),
                        });
                    }
                }
            }
            if let Some(rest) = trimmed.strip_prefix("fn test_") {
                let name = rest.split('(').next().unwrap_or(rest).trim();
                if !name.is_empty() {
                    tests.push(TestRefRecord {
                        path: f.path.clone(),
                        name: format!("test_{name}"),
                        language: f.language.clone(),
                    });
                }
            }
            if is_test_file {
                if let Some(rest) = trimmed.strip_prefix("func Test") {
                    let name = rest.split('(').next().unwrap_or(rest).trim();
                    if !name.is_empty() {
                        tests.push(TestRefRecord {
                            path: f.path.clone(),
                            name: format!("Test{name}"),
                            language: f.language.clone(),
                        });
                    }
                }
            }
            for prefix in ["\"/api/", "\"/v1/", "\"/cursor/"] {
                if let Some(idx) = trimmed.find(prefix) {
                    let rest = &trimmed[idx + 1..];
                    if let Some(end) = rest.find('\"') {
                        let route = &rest[..end];
                        if route.starts_with('/') && route.len() < 120 {
                            routes.push(ConfigKeyRecord {
                                path: f.path.clone(),
                                key: route.to_string(),
                                kind: "route".into(),
                            });
                        }
                    }
                }
            }
        }
    }
    configs.truncate(2000);
    tests.truncate(2000);
    routes.truncate(1000);
    (configs, tests, routes)
}

fn write_index_jsonl(path: &Path, rows: &[impl Serialize]) {
    let mut out = String::new();
    for r in rows {
        if let Ok(line) = serde_json::to_string(r) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = crate::fsutil::atomic_write_str(path, &out);
}

fn load_manifest(paths: &ProjectLearningPaths) -> IndexManifest {
    learning_store::read_json(&paths.index_dir.join("manifest.json")).unwrap_or_default()
}

fn save_manifest(paths: &ProjectLearningPaths, m: &IndexManifest) {
    let _ = learning_store::write_json_atomic(&paths.index_dir.join("manifest.json"), m);
}

/// Full or incremental index refresh. Returns `(files_indexed, symbols_indexed)`.
/// Fail-open: returns `(0,0)` on hard errors.
pub fn refresh_index(workspace: &Path, identity: &ProjectIdentity, force_full: bool) -> (usize, usize) {
    let paths = learning_store::ensure_project_learning(
        &identity.id,
        identity.remote.as_deref(),
        Some(&identity.workspace_hash),
    );
    let _lock = crate::fsutil::FileLock::acquire(&paths.index_dir.join("index.lock"));

    let mut manifest = load_manifest(&paths);
    if manifest.project_id.is_empty() {
        manifest.project_id = identity.id.clone();
        manifest.schema_version = MANIFEST_SCHEMA;
    }

    let source_files = collect_source_files(workspace);
    let mut files: Vec<FileRecord> = Vec::new();
    let mut symbols: Vec<SymbolRecord> = Vec::new();
    let mut relations: Vec<RelationRecord> = Vec::new();
    let mut new_hashes: HashMap<String, String> = HashMap::new();

    // Load previous records when incremental so unchanged files keep symbols.
    let prev_files: HashMap<String, FileRecord> = if force_full {
        HashMap::new()
    } else {
        learning_store::read_jsonl::<FileRecord>(&paths.index_dir.join("files.jsonl"))
            .into_iter()
            .map(|f| (f.path.clone(), f))
            .collect()
    };
    let prev_symbols: Vec<SymbolRecord> = if force_full {
        Vec::new()
    } else {
        learning_store::read_jsonl(&paths.index_dir.join("symbols.jsonl"))
    };
    let prev_relations: Vec<RelationRecord> = if force_full {
        Vec::new()
    } else {
        learning_store::read_jsonl(&paths.index_dir.join("relations.jsonl"))
    };
    let mut keep_symbol_paths: HashSet<String> = HashSet::new();

    for abs in &source_files {
        let rel = abs
            .strip_prefix(workspace)
            .unwrap_or(abs)
            .to_string_lossy()
            .replace('\\', "/");
        // Fast path: mtime+size cheap check via re-hash only when needed.
        let Some((file, syms, rels)) = index_one_file(workspace, abs) else {
            continue;
        };
        let unchanged = !force_full
            && manifest
                .file_hashes
                .get(&rel)
                .map(|h| h == &file.content_hash)
                .unwrap_or(false);
        new_hashes.insert(rel.clone(), file.content_hash.clone());
        if unchanged {
            if let Some(prev) = prev_files.get(&rel) {
                files.push(prev.clone());
            } else {
                files.push(file);
            }
            keep_symbol_paths.insert(rel);
            continue;
        }
        files.push(file);
        symbols.extend(syms);
        relations.extend(rels);
    }

    // Re-attach unchanged symbols/relations.
    for s in prev_symbols {
        if keep_symbol_paths.contains(&s.path) {
            symbols.push(s);
        }
    }
    for r in prev_relations {
        if keep_symbol_paths.contains(&r.from_path) {
            relations.push(r);
        }
    }

    write_index_jsonl(&paths.index_dir.join("files.jsonl"), &files);
    write_index_jsonl(&paths.index_dir.join("symbols.jsonl"), &symbols);
    write_index_jsonl(&paths.index_dir.join("relations.jsonl"), &relations);
    let (configs, tests, routes) = extract_config_tests_routes(workspace, &files);
    write_index_jsonl(&paths.index_dir.join("config.jsonl"), &configs);
    write_index_jsonl(&paths.index_dir.join("tests.jsonl"), &tests);
    write_index_jsonl(&paths.index_dir.join("routes.jsonl"), &routes);

    let now = now_secs();
    if force_full || manifest.last_full_scan_at == 0 {
        manifest.last_full_scan_at = now;
    }
    manifest.last_incremental_at = now;
    manifest.file_hashes = new_hashes;
    manifest.file_count = files.len();
    manifest.symbol_count = symbols.len();
    save_manifest(&paths, &manifest);

    (files.len(), symbols.len())
}

/// Look up symbols by exact name (case-sensitive).
pub fn find_symbols(project_id: &str, name: &str) -> Vec<SymbolRecord> {
    let paths = ProjectLearningPaths::resolve(project_id);
    learning_store::read_jsonl::<SymbolRecord>(&paths.index_dir.join("symbols.jsonl"))
        .into_iter()
        .filter(|s| s.name == name)
        .collect()
}

/// Files that import / reference `name`.
pub fn find_relations_to(project_id: &str, name: &str) -> Vec<RelationRecord> {
    let paths = ProjectLearningPaths::resolve(project_id);
    learning_store::read_jsonl::<RelationRecord>(&paths.index_dir.join("relations.jsonl"))
        .into_iter()
        .filter(|r| r.to_name.contains(name))
        .collect()
}

/// List indexed files.
pub fn list_files(project_id: &str) -> Vec<FileRecord> {
    let paths = ProjectLearningPaths::resolve(project_id);
    learning_store::read_jsonl(&paths.index_dir.join("files.jsonl"))
}

/// Ensure index exists / is incrementally fresh (session-start hook).
pub fn ensure_index(workspace: &Path) -> (String, usize, usize) {
    let identity = crate::project_identity::resolve_project_identity(workspace);
    let paths = ProjectLearningPaths::resolve(&identity.id);
    let manifest = load_manifest(&paths);
    let force = manifest.file_count == 0 || manifest.schema_version != MANIFEST_SCHEMA;
    // Throttle session-start refreshes so prompt/turn loops stay snappy.
    if !force
        && manifest.file_count > 0
        && now_secs().saturating_sub(manifest.last_incremental_at) < 30
    {
        return (identity.id, manifest.file_count, manifest.symbol_count);
    }
    let (f, s) = refresh_index(workspace, &identity, force);
    (identity.id, f, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning_store::override_learning_root;
    use crate::project_identity::override_registry_path;
    use std::sync::{Mutex, OnceLock};

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn tmp() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!(
            "codebase-index-{}-{}-{}",
            std::process::id(),
            now_secs(),
            n
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn indexes_rust_symbols_and_skips_binary() {
        let _serial = serial();
        let home = tmp();
        let _lserial = crate::learning_store::learning_test_serial().lock().unwrap_or_else(|e| e.into_inner());
        let _lr = override_learning_root(home.join("learning"));
        let _rserial = crate::project_identity::registry_test_serial().lock().unwrap_or_else(|e| e.into_inner());
        let _rr = override_registry_path(home.join("registry.json"));
        let ws = tmp();
        std::fs::create_dir_all(ws.join("core/src")).unwrap();
        std::fs::write(
            ws.join("core/src/lib.rs"),
            "use std::path::Path;\npub struct Foo {}\npub fn bar() {}\n",
        )
        .unwrap();
        std::fs::write(ws.join("core/src/blob.bin"), [0u8, 1, 2, 0, 9]).unwrap();
        std::fs::create_dir_all(ws.join("target/debug")).unwrap();
        std::fs::write(ws.join("target/debug/x.rs"), "fn ignored() {}").unwrap();

        let identity = crate::project_identity::resolve_project_identity(&ws);
        let (files, syms) = refresh_index(&ws, &identity, true);
        assert!(files >= 1);
        assert!(syms >= 2, "expected struct+fn, got {syms}");
        let listed = list_files(&identity.id);
        assert!(listed.iter().any(|f| f.path.ends_with("lib.rs")));
        assert!(!listed.iter().any(|f| f.path.contains("target/")), "target skipped");
        let foos = find_symbols(&identity.id, "Foo");
        assert_eq!(foos.len(), 1);
        assert_eq!(foos[0].kind, "struct");
        let imports = find_relations_to(&identity.id, "std::path::Path");
        assert!(!imports.is_empty());
    }

    #[test]
    fn incremental_skips_unchanged_and_drops_deleted() {
        let _serial = serial();
        let home = tmp();
        let _lserial = crate::learning_store::learning_test_serial().lock().unwrap_or_else(|e| e.into_inner());
        let _lr = override_learning_root(home.join("learning"));
        let _rserial = crate::project_identity::registry_test_serial().lock().unwrap_or_else(|e| e.into_inner());
        let _rr = override_registry_path(home.join("registry.json"));
        let ws = tmp();
        std::fs::write(ws.join("a.rs"), "pub fn one() {}\n").unwrap();
        std::fs::write(ws.join("b.rs"), "pub fn two() {}\n").unwrap();
        let identity = crate::project_identity::resolve_project_identity(&ws);
        let (_, s1) = refresh_index(&ws, &identity, true);
        assert!(s1 >= 2);
        // Delete b.rs and change a.rs
        std::fs::remove_file(ws.join("b.rs")).unwrap();
        std::fs::write(ws.join("a.rs"), "pub fn one() {}\npub fn three() {}\n").unwrap();
        let (f2, s2) = refresh_index(&ws, &identity, false);
        let listed = list_files(&identity.id);
        assert_eq!(listed.len(), 1, "deleted files must leave the index: {listed:?}");
        assert_eq!(f2, listed.len());
        let names: Vec<_> = find_symbols(&identity.id, "two");
        assert!(names.is_empty(), "deleted file symbols must disappear");
        assert!(!find_symbols(&identity.id, "three").is_empty());
        assert!(s2 >= 2);
    }

    #[test]
    fn secrets_not_stored_as_symbol_values() {
        // Env-like assignments are not parsed as symbols; only decls are.
        let (syms, _) = extract_symbols_and_relations(
            ".env",
            "unknown",
            "API_KEY=super-secret-value\n# comment\n",
        );
        assert!(syms.is_empty());
        assert!(!format!("{syms:?}").contains("super-secret"));
    }
}
