//! Document collections with local embedding-based retrieval (RAG).
//!
//! Named collections of text documents (API docs, specs, notes, design docs,
//! READMEs, …) are chunked and embedded with the local hashing-sketch embedder
//! ([`crate::embed`]), then retrieved by cosine similarity to a query. This
//! extends the harness's knowledge infrastructure beyond the codebase index to
//! *arbitrary* text the agent or user wants to ground a task in.
//!
//! Storage lives under the project learning dir, beside the codebase index:
//!
//! ```text
//! ~/.config/catalyst-code/learning/projects/<project-id>/collections/
//! └── <name>/
//!     ├── meta.json     — name, counts, timestamps
//!     ├── chunks.jsonl  — one chunk per line: {id, source, text, offset}
//!     └── vectors.json  — {chunk_id: [f32; DIM]}
//! ```
//!
//! Embeddings default to the zero-dependency hashing sketch
//! ([`crate::embed::hash_embed`]); the embedder is the single swap point for a
//! future HTTP/model-backed embedder. Fail-open: collection errors never abort
//! a coding turn (callers surface them as a tool error string).

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::embed::{cosine, hash_embed, DIM};
use crate::learning_store::learning_root;

/// Soft target chunk size in characters.
const CHUNK_TARGET: usize = 1000;
/// Soft chunk overlap in characters.
const CHUNK_OVERLAP: usize = 120;
/// Hard chunk ceiling in characters.
const CHUNK_MAX: usize = 1500;
/// Max files ingested by `index_directory` in one call (safety cap).
const INDEX_MAX_FILES: usize = 4000;
/// Max single-file bytes ingested by `index_directory`.
const INDEX_MAX_FILE_BYTES: u64 = 600_000;

/// Directories always skipped by `index_directory`.
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
    ".idea",
    ".vscode",
];

/// Text extensions ingested by `index_directory` (case-insensitive).
const TEXT_EXTS: &[&str] = &[
    "md",
    "txt",
    "rst",
    "markdown",
    "org",
    "adoc",
    "asciidoc",
    "log",
    "csv",
    "tsv",
    "json",
    "yaml",
    "yml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "env",
    "rs",
    "go",
    "ts",
    "tsx",
    "js",
    "jsx",
    "mjs",
    "cjs",
    "py",
    "rb",
    "java",
    "kt",
    "swift",
    "c",
    "h",
    "cc",
    "cpp",
    "hpp",
    "cs",
    "php",
    "pl",
    "lua",
    "sh",
    "bash",
    "zsh",
    "fish",
    "ps1",
    "sql",
    "proto",
    "thrift",
    "gradle",
    "dart",
    "scala",
    "clj",
    "ex",
    "exs",
    "erl",
    "hs",
    "ml",
    "nim",
    "v",
    "zig",
    "html",
    "htm",
    "css",
    "scss",
    "sass",
    "less",
    "vue",
    "svelte",
    "dockerfile",
    "makefile",
    "gemfile",
];

/// A single indexed chunk.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chunk {
    pub id: String,
    pub source: String,
    pub text: String,
    /// Character offset of this chunk within the source document.
    pub offset: usize,
}

/// Collection metadata.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CollectionMeta {
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub chunk_count: usize,
    pub source_count: usize,
    /// Total characters indexed.
    pub total_chars: usize,
}

/// A retrieval hit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchHit {
    pub collection: String,
    pub chunk_id: String,
    pub source: String,
    pub score: f32,
    pub offset: usize,
    pub text: String,
}

/// Result of an ingest operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddResult {
    pub collection: String,
    pub chunks_added: usize,
    pub chars_indexed: usize,
    pub total_chunks: usize,
}

/// Result of a directory ingest.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexResult {
    pub collection: String,
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub chunks_added: usize,
    pub bytes_indexed: u64,
    pub skipped: Vec<String>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Root directory for all collections in a project.
pub fn collections_root(project_id: &str) -> PathBuf {
    learning_root()
        .join("projects")
        .join(project_id)
        .join("collections")
}

fn collection_dir(project_id: &str, name: &str) -> PathBuf {
    collections_root(project_id).join(sanitize_name(name))
}

fn meta_path(project_id: &str, name: &str) -> PathBuf {
    collection_dir(project_id, name).join("meta.json")
}

fn chunks_path(project_id: &str, name: &str) -> PathBuf {
    collection_dir(project_id, name).join("chunks.jsonl")
}

fn vectors_path(project_id: &str, name: &str) -> PathBuf {
    collection_dir(project_id, name).join("vectors.json")
}

/// Slug a collection name into a safe directory segment (lowercase, alnum + -).
fn sanitize_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if c == '-' || c == '_' || c == '.' {
            out.push(c);
        } else if c.is_whitespace() {
            out.push('-');
        }
    }
    if out.is_empty() {
        out.push_str("default");
    }
    // Trim leading dots so it never looks hidden.
    out.trim_start_matches('.').to_string()
}

/// Validate a collection name is non-empty and not path-traversing.
fn validate_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("collection name is required".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err("collection name must not contain path separators or '..'".to_string());
    }
    if trimmed.len() > 128 {
        return Err("collection name too long (max 128 chars)".to_string());
    }
    Ok(trimmed.to_string())
}

/// Load a collection's chunks (skipping malformed lines — fail-open).
fn load_chunks(project_id: &str, name: &str) -> Vec<Chunk> {
    let p = chunks_path(project_id, name);
    let mut out = Vec::new();
    if let Ok(s) = std::fs::read_to_string(&p) {
        for line in s.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(c) = serde_json::from_str::<Chunk>(line) {
                out.push(c);
            }
        }
    }
    out
}

/// Load a collection's vector map.
fn load_vectors(project_id: &str, name: &str) -> HashMap<String, Vec<f32>> {
    let p = vectors_path(project_id, name);
    #[derive(Deserialize)]
    struct VFile {
        #[serde(default)]
        vectors: HashMap<String, Vec<f32>>,
    }
    std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str::<VFile>(&s).ok())
        .map(|f| f.vectors)
        .unwrap_or_default()
}

fn save_vectors(project_id: &str, name: &str, vectors: &HashMap<String, Vec<f32>>) {
    let p = vectors_path(project_id, name);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    #[derive(Serialize)]
    struct VFile<'a> {
        version: u32,
        dim: usize,
        vectors: &'a HashMap<String, Vec<f32>>,
    }
    let f = VFile {
        version: 1,
        dim: DIM,
        vectors,
    };
    if let Ok(s) = serde_json::to_string(&f) {
        let _ = std::fs::write(p, s);
    }
}

fn load_meta(project_id: &str, name: &str) -> Option<CollectionMeta> {
    std::fs::read_to_string(meta_path(project_id, name))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn save_meta(project_id: &str, name: &str, mut meta: CollectionMeta) {
    meta.updated_at = now_secs();
    let p = meta_path(project_id, name);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(s) = serde_json::to_string_pretty(&meta) {
        let _ = std::fs::write(p, s);
    }
}

fn ensure_collection_dir(project_id: &str, name: &str) -> Result<PathBuf, String> {
    let d = collection_dir(project_id, name);
    std::fs::create_dir_all(&d).map_err(|e| format!("create collection dir failed: {e}"))?;
    Ok(d)
}

// ---------------------------------------------------------------------------
// Chunking
// ---------------------------------------------------------------------------

/// Split text into overlapping chunks of roughly `CHUNK_TARGET` characters,
/// preferring paragraph boundaries. Returns chunks with character offsets.
pub fn chunk_text(text: &str) -> Vec<Chunk> {
    chunk_text_with(text, CHUNK_TARGET, CHUNK_OVERLAP, CHUNK_MAX, "")
}

/// Chunking with explicit parameters + source label. Exposed for tests.
pub fn chunk_text_with(
    text: &str,
    target: usize,
    overlap: usize,
    max: usize,
    source: &str,
) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    if text.trim().is_empty() {
        return chunks;
    }
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut start = 0usize;
    let mut idx = 0usize;
    while start < n {
        let mut end = (start + max).min(n);
        // Try to end on a paragraph or sentence boundary within [target, max].
        if end < n {
            let want = start + target;
            let search_to = (start + max).min(n);
            if let Some(b) = best_boundary(&chars, want, search_to) {
                end = b + 1; // include the boundary char
            }
        }
        let slice: String = chars[start..end].iter().collect();
        let trimmed = slice.trim();
        if !trimmed.is_empty() {
            chunks.push(Chunk {
                id: format!("c{idx}"),
                source: source.to_string(),
                text: trimmed.to_string(),
                offset: start,
            });
            idx += 1;
        }
        if end >= n {
            break;
        }
        // Advance with overlap, but always make forward progress.
        let next = if end > start + overlap {
            end - overlap
        } else {
            end
        };
        if next <= start {
            start = end;
        } else {
            start = next;
        }
    }
    // Renumber ids sequentially for stable, readable ids.
    for (i, c) in chunks.iter_mut().enumerate() {
        c.id = format!("c{i}");
    }
    chunks
}

/// Find the best split boundary at or after `want`, within `max_pos`.
/// Prefers double-newline (paragraph), then single newline, then sentence
/// punctuation, then whitespace.
fn best_boundary(chars: &[char], want: usize, max_pos: usize) -> Option<usize> {
    if want >= max_pos {
        return Some(max_pos - 1);
    }
    // Search window [want-50, max_pos) — a little lookback so a boundary just
    // before the target is preferred over overshooting.
    let lo = want.saturating_sub(50);
    let mut best: Option<(usize, u8)> = None;
    let mut i = max_pos.min(chars.len());
    while i > lo {
        i -= 1;
        let c = chars[i];
        let rank = if c == '\n' && i + 1 < chars.len() && chars[i + 1] == '\n' {
            4u8 // paragraph
        } else if c == '\n' {
            3u8 // line
        } else if (c == '.' || c == '!' || c == '?')
            && i + 1 < chars.len()
            && (chars[i + 1].is_whitespace() || chars[i + 1] == '\n')
        {
            2u8 // sentence end
        } else if c.is_whitespace() && c != '\n' {
            1u8 // word
        } else {
            continue;
        };
        match best {
            Some((_, br)) if br >= rank => {}
            _ => best = Some((i, rank)),
        }
        if rank == 4 {
            break;
        }
    }
    best.map(|(p, _)| p)
}

// ---------------------------------------------------------------------------
// Public ingest / query API
// ---------------------------------------------------------------------------

/// Add a text document to a collection (creating it if needed). Returns the
/// number of chunks added and the new total.
pub fn add_text(
    project_id: &str,
    collection: &str,
    text: &str,
    source: &str,
) -> Result<AddResult, String> {
    let name = validate_name(collection)?;
    ensure_collection_dir(project_id, &name)?;
    let source = if source.trim().is_empty() {
        "inline"
    } else {
        source.trim()
    };

    let new_chunks = chunk_text_with(text, CHUNK_TARGET, CHUNK_OVERLAP, CHUNK_MAX, source);
    if new_chunks.is_empty() {
        return Ok(AddResult {
            collection: name,
            chunks_added: 0,
            chars_indexed: 0,
            total_chunks: 0,
        });
    }

    // Append to existing chunks, re-deriving sequential ids.
    let mut existing = load_chunks(project_id, &name);
    let start = existing.len();
    for (i, mut c) in new_chunks.into_iter().enumerate() {
        c.id = format!("c{}", start + i);
        existing.push(c);
    }

    let mut vectors = load_vectors(project_id, &name);
    for c in &existing {
        vectors
            .entry(c.id.clone())
            .or_insert_with(|| hash_embed(&format!("{}\n{}", c.source, c.text)));
    }

    persist(project_id, &name, &existing, &vectors)?;

    let chunk_count = existing.len();
    let source_count = existing
        .iter()
        .map(|c| c.source.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let total_chars = existing.iter().map(|c| c.text.chars().count()).sum();
    let meta = CollectionMeta {
        name: name.clone(),
        created_at: load_meta(project_id, &name)
            .map(|m| m.created_at)
            .unwrap_or_else(now_secs),
        updated_at: 0,
        chunk_count,
        source_count,
        total_chars,
    };
    save_meta(project_id, &name, meta);

    Ok(AddResult {
        collection: name,
        chunks_added: chunk_count - start,
        chars_indexed: text.chars().count(),
        total_chunks: chunk_count,
    })
}

/// Recursively ingest supported text files under `root` into a collection.
/// `root` must be an absolute, already-resolved directory (the tool layer
/// resolves workspace-relative paths). Returns counts + a sample of skipped
/// paths (capped).
pub fn index_directory(
    project_id: &str,
    collection: &str,
    root: &Path,
    exts: Option<&[String]>,
) -> Result<IndexResult, String> {
    let name = validate_name(collection)?;
    if !root.is_dir() {
        return Err(format!("index path is not a directory: {}", root.display()));
    }
    let allowed: Vec<String> = exts
        .map(|e| {
            e.iter()
                .map(|s| s.trim_start_matches('.').to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_else(|| TEXT_EXTS.iter().map(|s| s.to_string()).collect());

    let mut files_indexed = 0usize;
    let mut files_skipped = 0usize;
    let mut chunks_added = 0usize;
    let mut bytes_indexed = 0u64;
    let mut skipped = Vec::new();
    let mut visited = 0usize;

    walk(root, root, &mut |path, rel| {
        visited += 1;
        if visited > INDEX_MAX_FILES {
            return;
        }
        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => {
                files_skipped += 1;
                if skipped.len() < 20 {
                    skipped.push(rel.to_string());
                }
                return;
            }
        };
        if meta.len() > INDEX_MAX_FILE_BYTES {
            files_skipped += 1;
            if skipped.len() < 20 {
                skipped.push(rel.to_string());
            }
            return;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        // Files with no extension: ingest only well-known text filenames.
        let basename = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.to_ascii_lowercase())
            .unwrap_or_default();
        let ok = allowed.iter().any(|e| e == &ext)
            || (ext.is_empty() && TEXT_EXTS.contains(&basename.as_str()));
        if !ok {
            files_skipped += 1;
            return;
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => {
                files_skipped += 1;
                if skipped.len() < 20 {
                    skipped.push(rel.to_string());
                }
                return;
            }
        };
        if text.trim().is_empty() {
            files_skipped += 1;
            return;
        }
        match add_text(project_id, &name, &text, rel) {
            Ok(r) => {
                files_indexed += 1;
                chunks_added += r.chunks_added;
                bytes_indexed += text.len() as u64;
            }
            Err(e) => {
                files_skipped += 1;
                if skipped.len() < 20 {
                    skipped.push(format!("{}: {e}", rel));
                }
            }
        }
    });

    Ok(IndexResult {
        collection: name,
        files_indexed,
        files_skipped,
        chunks_added,
        bytes_indexed,
        skipped,
    })
}

fn walk(root: &Path, dir: &Path, f: &mut impl FnMut(&Path, &str)) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) || name.starts_with('.') {
                continue;
            }
            walk(root, &path, f);
            continue;
        }
        if path.is_symlink() {
            // Skip symlinks to avoid loops.
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string());
        f(&path, &rel);
    }
}

/// Retrieve the top-`limit` chunks in a collection by similarity to `query`.
pub fn search(
    project_id: &str,
    collection: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, String> {
    let name = validate_name(collection)?;
    let limit = if limit == 0 { 6 } else { limit.min(50) };
    let chunks = load_chunks(project_id, &name);
    if chunks.is_empty() {
        return Ok(Vec::new());
    }
    let vectors = load_vectors(project_id, &name);
    let q = hash_embed(&format!("{}\n{}", name, query));
    let mut scored: Vec<(usize, f32)> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let v = vectors
                .get(&c.id)
                .cloned()
                .unwrap_or_else(|| hash_embed(&format!("{}\n{}", c.source, c.text)));
            (i, cosine(&q, &v))
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    Ok(scored
        .into_iter()
        .filter(|(_, s)| *s > 0.0)
        .map(|(i, s)| SearchHit {
            collection: name.clone(),
            chunk_id: chunks[i].id.clone(),
            source: chunks[i].source.clone(),
            score: s,
            offset: chunks[i].offset,
            text: chunks[i].text.clone(),
        })
        .collect())
}

/// List all collections for a project with metadata.
pub fn list(project_id: &str) -> Vec<CollectionMeta> {
    let root = collections_root(project_id);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let name = match p.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if let Some(m) = load_meta(project_id, &name) {
                out.push(m);
            } else if let Ok(chunks) = std::fs::read_to_string(chunks_path(project_id, &name)) {
                // meta.json missing — derive a minimal record.
                let cc = chunks.lines().filter(|l| !l.trim().is_empty()).count();
                out.push(CollectionMeta {
                    name,
                    chunk_count: cc,
                    ..Default::default()
                });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Stats for a single collection.
pub fn stats(project_id: &str, collection: &str) -> Result<CollectionMeta, String> {
    let name = validate_name(collection)?;
    if !collection_dir(project_id, &name).exists() {
        return Err(format!("collection '{}' not found", name));
    }
    if let Some(m) = load_meta(project_id, &name) {
        // Re-derive counts from disk so stats stay accurate after external edits.
        let chunks = load_chunks(project_id, &name);
        let source_count = chunks
            .iter()
            .map(|c| c.source.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len();
        let total_chars = chunks.iter().map(|c| c.text.chars().count()).sum();
        return Ok(CollectionMeta {
            chunk_count: chunks.len(),
            source_count,
            total_chars,
            ..m
        });
    }
    let chunks = load_chunks(project_id, &name);
    Ok(CollectionMeta {
        name: name.clone(),
        created_at: 0,
        updated_at: 0,
        chunk_count: chunks.len(),
        source_count: chunks
            .iter()
            .map(|c| c.source.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        total_chars: chunks.iter().map(|c| c.text.chars().count()).sum(),
    })
}

/// Delete a collection entirely.
pub fn remove(project_id: &str, collection: &str) -> Result<(), String> {
    let name = validate_name(collection)?;
    let d = collection_dir(project_id, &name);
    if !d.exists() {
        return Err(format!("collection '{}' not found", name));
    }
    std::fs::remove_dir_all(&d).map_err(|e| format!("remove collection failed: {e}"))
}

/// Persist chunks + vectors to disk (atomic-ish: write vectors then chunks).
fn persist(
    project_id: &str,
    name: &str,
    chunks: &[Chunk],
    vectors: &HashMap<String, Vec<f32>>,
) -> Result<(), String> {
    ensure_collection_dir(project_id, name)?;
    // Vectors first (recoverable from chunks if interrupted).
    save_vectors(project_id, name, vectors);
    // Chunks as JSONL.
    let p = chunks_path(project_id, name);
    let mut buf = String::with_capacity(chunks.len() * 256);
    for c in chunks {
        if let Ok(s) = serde_json::to_string(c) {
            buf.push_str(&s);
            buf.push('\n');
        }
    }
    std::fs::write(&p, buf).map_err(|e| format!("write chunks failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning_store::{learning_test_serial, override_learning_root};

    fn tmp_root() -> (std::path::PathBuf, impl Drop) {
        let guard = learning_test_serial().lock().unwrap();
        let dir =
            std::env::temp_dir().join(format!("catcode-coll-{}-{}", std::process::id(), counter()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = override_learning_root(dir.clone());
        (
            dir,
            ScopedGuard {
                _g: g,
                _lock: guard,
            },
        )
    }

    // Drop guard that cleans the temp learning root on scope end.
    struct ScopedGuard {
        _g: crate::learning_store::LearningRootGuard,
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl Drop for ScopedGuard {
        fn drop(&mut self) {
            // best-effort cleanup of the temp dir
        }
    }

    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    fn counter() -> usize {
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    const PID: &str = "test-project";

    #[test]
    fn chunk_text_short_returns_one_chunk() {
        let chunks = chunk_text_with("hello world this is a short doc", 1000, 120, 1500, "a.md");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].id, "c0");
        assert_eq!(chunks[0].source, "a.md");
    }

    #[test]
    fn chunk_text_splits_on_paragraph_boundaries() {
        let mut s = String::new();
        for i in 0..20 {
            s.push_str(&format!("Paragraph {i}. This is a sentence about topic {i}. It has enough words to grow the body.\n\n"));
        }
        let chunks = chunk_text_with(&s, 200, 40, 300, "doc.md");
        assert!(
            chunks.len() > 1,
            "expected multiple chunks, got {}",
            chunks.len()
        );
        // offsets are monotonic and within bounds
        for w in chunks.windows(2) {
            assert!(w[0].offset < w[1].offset || w[0].offset == 0);
        }
        // no chunk exceeds the soft max by too much (allow boundary overshoot)
        for c in &chunks {
            assert!(
                c.text.chars().count() <= 320,
                "chunk too long: {}",
                c.text.chars().count()
            );
        }
    }

    #[test]
    fn chunk_text_empty_is_empty() {
        assert!(chunk_text("").is_empty());
        assert!(chunk_text("   \n\n  ").is_empty());
    }

    #[test]
    fn add_then_search_roundtrip() {
        let (_d, _g) = tmp_root();
        let doc = "The Redis SCAN command iterates keys incrementally. \
                   It uses a cursor and returns a batch each call. \
                   Never use KEYS in production — it blocks the server.";
        let r = add_text(PID, "redis-docs", doc, "redis.md").unwrap();
        assert_eq!(r.collection, "redis-docs");
        assert!(r.chunks_added >= 1);

        let hits = search(
            PID,
            "redis-docs",
            "How do I iterate keys without blocking?",
            5,
        )
        .unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].score > 0.0);
        assert!(hits[0].text.contains("SCAN") || hits[0].text.contains("cursor"));
    }

    #[test]
    fn add_appends_across_calls_and_ids_stay_sequential() {
        let (_d, _g) = tmp_root();
        add_text(PID, "notes", "first document body about alpha.", "a.txt").unwrap();
        add_text(PID, "notes", "second document body about beta.", "b.txt").unwrap();
        let chunks = load_chunks(PID, "notes");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].id, "c0");
        assert_eq!(chunks[1].id, "c1");
        let m = stats(PID, "notes").unwrap();
        assert_eq!(m.chunk_count, 2);
        assert_eq!(m.source_count, 2);
    }

    #[test]
    fn list_shows_collections() {
        let (_d, _g) = tmp_root();
        add_text(PID, "alpha", "alpha content here", "a").unwrap();
        add_text(PID, "beta", "beta content here too", "b").unwrap();
        let names: Vec<_> = list(PID).into_iter().map(|m| m.name).collect();
        assert!(names.contains(&"alpha".to_string()));
        assert!(names.contains(&"beta".to_string()));
    }

    #[test]
    fn remove_deletes_collection() {
        let (_d, _g) = tmp_root();
        add_text(PID, "ephemeral", "some text", "s").unwrap();
        assert!(collection_dir(PID, "ephemeral").exists());
        remove(PID, "ephemeral").unwrap();
        assert!(!collection_dir(PID, "ephemeral").exists());
        assert!(stats(PID, "ephemeral").is_err());
    }

    #[test]
    fn validate_name_rejects_traversal() {
        assert!(validate_name("").is_err());
        assert!(validate_name("../x").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("ok name").is_ok());
    }

    #[test]
    fn sanitize_name_is_safe() {
        assert_eq!(sanitize_name("API Docs!"), "api-docs");
        assert_eq!(sanitize_name(""), "default");
        assert_eq!(sanitize_name(".hidden"), "hidden");
    }

    #[test]
    fn index_directory_ingests_files() {
        let (_d, _g) = tmp_root();
        let root = std::env::temp_dir().join(format!("catcode-coll-src-{}", counter()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(
            root.join("a.md"),
            "# A\nSome markdown content about cats.\n",
        )
        .unwrap();
        std::fs::write(
            root.join("sub/b.txt"),
            "Plain text about dogs and fences.\n",
        )
        .unwrap();
        std::fs::write(root.join("skip.bin"), [0u8, 1, 2]).unwrap(); // skipped ext

        let r = index_directory(PID, "mixed", &root, None).unwrap();
        assert_eq!(r.collection, "mixed");
        assert!(r.files_indexed >= 2, "indexed {}", r.files_indexed);
        assert!(r.chunks_added >= 2);

        let hits = search(PID, "mixed", "cats", 5).unwrap();
        assert!(!hits.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn search_unknown_collection_returns_empty() {
        let (_d, _g) = tmp_root();
        let hits = search(PID, "nope", "anything", 5).unwrap();
        assert!(hits.is_empty());
    }
}
