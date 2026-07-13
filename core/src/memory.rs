// persistent memory system. Stores named memories as markdown files with
// YAML-like frontmatter under ~/.config/catalyst-code/memory/<project-hash>/.
// Memories are scoped per workspace (hashed canonical path) and injected into
// the standing system prompt so learnings persist across sessions.
// ponytail: no DB, no extra crate — just markdown files on disk.
//
// Wired end-to-end: the `memory` AI tool (tools.rs) exposes save/append/list/
// forget to the model; the TUI slash commands (/remember /memory /forget) map
// to the SaveMemory/ListMemory/ForgetMemory core commands; memory_injection is
// spliced into the system prompt (main.rs). append_memory also runs at
// compaction to preserve durable facts. `project_hash` is a standalone helper
// kept for potential external use, hence the module-level dead-code allow.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Mutex as StdMutex;
use std::time::Instant;

/// Serializes all memory write operations (save/append/forget) across the
/// orchestrator and any in-process subagents. `append_memory` is a
/// read-modify-write, so two parallel subagents appending to the same memory
/// name would otherwise race and silently drop facts. Writes are rare and fast,
/// so a single global lock is the ponytail fix (no per-file lock map) and is
/// correct for one core process — the only writer to a memory dir.
static WRITE_LOCK: StdMutex<()> = StdMutex::new(());

/// Optional override for the memory store root (tests only). Avoids mutating
/// process-global `HOME`, which races with parallel tests that also touch
/// the memory store.
static ROOT_OVERRIDE: StdMutex<Option<PathBuf>> = StdMutex::new(None);

/// Process-local scan + relevant-tail cache. `scan_all_memories` reads every
/// `.md` on each call; without this, every model round (including post-tool
/// re-streams) re-reads the whole store. Invalidated on any write.
struct MemoryScanCache {
    /// Workspace project hash this cache belongs to (empty = unset).
    ws_hash: String,
    entries: Vec<MemoryEntry>,
    /// `(prompt, rendered tail)` for the current user turn.
    relevant: Option<(String, String)>,
    /// Wall time of last successful scan (tests / debugging).
    scanned_at: Option<Instant>,
}

impl MemoryScanCache {
    const fn empty() -> Self {
        Self {
            ws_hash: String::new(),
            entries: Vec::new(),
            relevant: None,
            scanned_at: None,
        }
    }
}

static SCAN_CACHE: StdMutex<MemoryScanCache> = StdMutex::new(MemoryScanCache::empty());

/// Drop cached scans / relevant tails. Called after every successful memory
/// mutation so the next request re-reads from disk.
pub fn invalidate_scan_cache() {
    if let Ok(mut c) = SCAN_CACHE.lock() {
        *c = MemoryScanCache::empty();
    }
}

/// Serializes tests that touch the default memory store or install a root
/// override — without this, parallel `tools` memory tests race with hygiene
/// tests that temporarily redirect the store root.
#[cfg(test)]
pub fn memory_test_serial() -> &'static StdMutex<()> {
    use std::sync::OnceLock;
    static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| StdMutex::new(()))
}

/// RAII guard that installs a temporary memory root and restores the previous
/// override on drop. Tests that need an isolated store should hold this guard
/// for the duration of the test body.
pub struct MemoryRootGuard {
    prev: Option<PathBuf>,
}

impl Drop for MemoryRootGuard {
    fn drop(&mut self) {
        let mut g = ROOT_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner());
        *g = self.prev.take();
    }
}

/// Install `root` as the memory store root until the returned guard is dropped.
#[cfg(test)]
pub fn override_memory_root(root: PathBuf) -> MemoryRootGuard {
    let mut g = ROOT_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner());
    let prev = g.replace(root);
    MemoryRootGuard { prev }
}

fn memory_store_root() -> PathBuf {
    if let Ok(g) = ROOT_OVERRIDE.lock() {
        if let Some(ref p) = *g {
            return p.clone();
        }
    }
    let home = crate::config::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".config/catalyst-code/memory")
}

/// Memory scope: workspace-local (per-codebase) or global (cross-codebase).
/// Global memories carry user-level facts — the user's name, preferred tech
/// stacks, harness conventions — that apply regardless of which project is
/// open. They are stored in a fixed `global/` directory and merged into every
/// workspace's system-prompt injection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Workspace,
    Global,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Scope::Workspace => "workspace",
            Scope::Global => "global",
        }
    }

    /// Parse a scope string; unrecognized values default to Workspace.
    pub fn parse(s: &str) -> Scope {
        match s.trim().to_lowercase().as_str() {
            "global" | "user" => Scope::Global,
            _ => Scope::Workspace,
        }
    }
}

/// Relative durability hint for catalog preference + write policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Importance {
    High,
    #[default]
    Normal,
    Low,
}

impl Importance {
    pub fn as_str(self) -> &'static str {
        match self {
            Importance::High => "high",
            Importance::Normal => "normal",
            Importance::Low => "low",
        }
    }

    pub fn parse(s: &str) -> Importance {
        match s.trim().to_lowercase().as_str() {
            "high" | "critical" | "durable" => Importance::High,
            "low" | "ephemeral" | "temp" => Importance::Low,
            _ => Importance::Normal,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MemoryEntry {
    pub name: String,
    pub description: String,
    pub mem_type: String,
    pub content: String,
    pub path: PathBuf,
    pub scope: Scope,
    /// When true (frontmatter `pin: true`), this memory is preferred in the
    /// standing catalog over unpinned notes when the entry budget is tight.
    pub pinned: bool,
    /// Frontmatter `importance:` (high|normal|low). High ranks with pins in
    /// the catalog; low is discouraged by the write policy unless forced.
    pub importance: Importance,
    /// When true, this memory is superseded/invalidated and excluded from the
    /// standing catalog + per-turn relevant tail (the successor carries the
    /// knowledge). Set via `memory save ... replaces=` / `memory deprecate`;
    /// still visible via `list`/`get`/`forget` so it can be audited.
    pub deprecated: bool,
    /// Name/id of the memory that supersedes this one (frontmatter
    /// `superseded_by`), set when a new memory `replaces` this one.
    pub superseded_by: Option<String>,
}

/// Standing-prompt catalog caps. Bodies are NOT injected — only name/type/scope
/// + one-line description — so a large store stays cheap in the prefix cache.
/// Full text is loaded on demand via `memory` action=get (or list).
pub const CATALOG_MAX_ENTRIES: usize = 48;
/// ~2.5k tokens at the chars/4 heuristic used elsewhere in the harness.
pub const CATALOG_MAX_CHARS: usize = 10_000;
const CATALOG_DESC_MAX_CHARS: usize = 100;
/// Maximum pinned entries shown in the standing catalog, so a large set of
/// pinned convention/decision memories can't crowd out operational
/// architecture/note/gotcha knowledge. Pinned entries beyond this budget are
/// omitted (still visible via `list`/`get`).
pub const CATALOG_PIN_BUDGET: usize = 16;

/// Per-turn relevant-memory tail (transient, not prefix-cached).
pub const RELEVANT_MAX_ENTRIES: usize = 8;
const RELEVANT_PREVIEW_LINES: usize = 5;

/// Soft warning threshold for the `memory` tool after save/append.
pub const SAVE_COUNT_WARN_THRESHOLD: usize = 60;

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
        memory_store_root()
    }

    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn dir(&self, workspace: &Path) -> PathBuf {
        self.root.join(hash_workspace(workspace))
    }

    /// Fixed directory for global (cross-workspace) memories.
    fn global_dir(&self) -> PathBuf {
        self.root.join("global")
    }

    /// Resolve the memory directory for a given scope.
    fn dir_scoped(&self, workspace: &Path, scope: Scope) -> PathBuf {
        match scope {
            Scope::Global => self.global_dir(),
            Scope::Workspace => self.dir(workspace),
        }
    }

    fn scan(&self, workspace: &Path) -> Vec<MemoryEntry> {
        scan_dir(&self.dir(workspace), Scope::Workspace)
    }

    fn scan_scoped(&self, workspace: &Path, scope: Scope) -> Vec<MemoryEntry> {
        scan_dir(&self.dir_scoped(workspace, scope), scope)
    }

    fn save(
        &self,
        workspace: &Path,
        name: &str,
        content: &str,
        mem_type: &str,
        description: &str,
    ) -> Result<PathBuf, String> {
        self.save_scoped(
            workspace,
            Scope::Workspace,
            name,
            content,
            mem_type,
            description,
        )
    }

    fn save_scoped(
        &self,
        workspace: &Path,
        scope: Scope,
        name: &str,
        content: &str,
        mem_type: &str,
        description: &str,
    ) -> Result<PathBuf, String> {
        self.save_scoped_with_importance(
            workspace,
            scope,
            name,
            content,
            mem_type,
            description,
            Importance::Normal,
        )
    }

    fn save_scoped_with_importance(
        &self,
        workspace: &Path,
        scope: Scope,
        name: &str,
        content: &str,
        mem_type: &str,
        description: &str,
        importance: Importance,
    ) -> Result<PathBuf, String> {
        let dir = self.dir_scoped(workspace, scope);
        std::fs::create_dir_all(&dir).map_err(|e| format!("failed to create memory dir: {e}"))?;

        let slug = slugify(name);
        if slug.is_empty() {
            return Err("memory name must contain at least one alphanumeric character".to_string());
        }
        let filename = format!("{}.md", slug);
        let path = dir.join(&filename);

        let pin_line = if is_pinned_type(mem_type) || importance == Importance::High {
            "pin: true\n"
        } else {
            ""
        };
        let importance_line = if importance != Importance::Normal {
            format!("importance: {}\n", importance.as_str())
        } else {
            String::new()
        };
        let body = format!(
            "---\nname: {}\ndescription: {}\ntype: {}\n{pin_line}{importance_line}---\n{}",
            name, description, mem_type, content
        );

        // Atomic + fsync'd write (temp + fsync + rename) so a crash mid-write
        // can't leave a truncated/empty memory file — memories are durable
        // learnings, so they get the same crash-safety as session persistence.
        atomic_write(&path, &body)
            .map_err(|e| format!("failed to write memory file {filename:?}: {e}"))?;

        rebuild_index(&dir, scope)?;

        Ok(path)
    }
}

// ---- public API ----

/// Scan all memory files for a workspace, returning parsed entries.
/// Skips the index file (MEMORY.md) and any unparseable files.
pub fn scan_memories(workspace: &Path) -> Vec<MemoryEntry> {
    scan_memories_scoped(workspace, Scope::Workspace)
}

/// Like `scan_memories` but for a specific scope.
pub fn scan_memories_scoped(workspace: &Path, scope: Scope) -> Vec<MemoryEntry> {
    Store::new(Store::default_root()).scan_scoped(workspace, scope)
}

/// Scan memories from BOTH scopes: global first (user-level, cross-codebase
/// facts), then workspace (project-specific). Each entry's `scope` field
/// identifies its origin. Used by `memory_injection` so the system prompt
/// carries forward both universal and project-specific learnings.
///
/// Results are process-cached per workspace hash and invalidated on write.
pub fn scan_all_memories(workspace: &Path) -> Vec<MemoryEntry> {
    let hash = hash_workspace(workspace);
    {
        let cache = SCAN_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if cache.ws_hash == hash && cache.scanned_at.is_some() {
            return cache.entries.clone();
        }
    }
    let store = Store::new(Store::default_root());
    let mut entries = store.scan_scoped(workspace, Scope::Global);
    entries.extend(store.scan_scoped(workspace, Scope::Workspace));
    {
        let mut cache = SCAN_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        cache.ws_hash = hash;
        cache.entries = entries.clone();
        cache.relevant = None;
        cache.scanned_at = Some(Instant::now());
    }
    entries
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
    save_memory_scoped(
        workspace,
        Scope::Workspace,
        name,
        content,
        mem_type,
        description,
    )
}

/// Like `save_memory` but for a specific scope. Use `Scope::Global` to store a
/// cross-codebase memory (user identity, tech-stack preferences, harness facts)
/// that is injected into every workspace's system prompt.
pub fn save_memory_scoped(
    workspace: &Path,
    scope: Scope,
    name: &str,
    content: &str,
    mem_type: &str,
    description: &str,
) -> Result<PathBuf, String> {
    save_memory_scoped_with_importance(
        workspace,
        scope,
        name,
        content,
        mem_type,
        description,
        Importance::Normal,
    )
}

/// Like `save_memory_scoped` but records an importance hint in frontmatter.
pub fn save_memory_scoped_with_importance(
    workspace: &Path,
    scope: Scope,
    name: &str,
    content: &str,
    mem_type: &str,
    description: &str,
    importance: Importance,
) -> Result<PathBuf, String> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = Store::new(Store::default_root()).save_scoped_with_importance(
        workspace,
        scope,
        name,
        content,
        mem_type,
        description,
        importance,
    )?;
    // Drop after write succeeds so the next scan/relevance call re-reads disk.
    drop(_guard);
    invalidate_scan_cache();
    Ok(path)
}

/// Append `new_facts` to an existing memory (same name/slug), capped to
/// `max_bytes` by trimming the oldest facts from the front (on a line boundary).
/// Unlike `save_memory` (which overwrites), this accumulates durable facts across
/// compactions so early-session facts aren't lost when later compactions fire —
/// the rolling cap keeps the file bounded instead of growing forever.
pub fn append_memory(
    workspace: &Path,
    name: &str,
    new_facts: &str,
    mem_type: &str,
    description: &str,
    max_bytes: usize,
) -> Result<PathBuf, String> {
    append_memory_scoped(
        workspace,
        Scope::Workspace,
        name,
        new_facts,
        mem_type,
        description,
        max_bytes,
    )
}

/// Like `append_memory` but for a specific scope. Use `Scope::Global` to
/// accumulate cross-codebase facts.
pub fn append_memory_scoped(
    workspace: &Path,
    scope: Scope,
    name: &str,
    new_facts: &str,
    mem_type: &str,
    description: &str,
    max_bytes: usize,
) -> Result<PathBuf, String> {
    append_memory_locked(
        &Store::new(Store::default_root()),
        workspace,
        scope,
        name,
        new_facts,
        mem_type,
        description,
        max_bytes,
    )
}

/// Like `append_memory` but against a provided store, and the testable seam for
/// the write lock: acquires `WRITE_LOCK` across the whole read-modify-write so
/// concurrent appends to the same memory name (e.g. from parallel subagents)
/// can't interleave and drop facts.
fn append_memory_locked(
    store: &Store,
    workspace: &Path,
    scope: Scope,
    name: &str,
    new_facts: &str,
    mem_type: &str,
    description: &str,
    max_bytes: usize,
) -> Result<PathBuf, String> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = append_memory_into(
        store,
        workspace,
        scope,
        name,
        new_facts,
        mem_type,
        description,
        max_bytes,
    )?;
    drop(_guard);
    invalidate_scan_cache();
    Ok(path)
}

fn append_memory_into(
    store: &Store,
    workspace: &Path,
    scope: Scope,
    name: &str,
    new_facts: &str,
    mem_type: &str,
    description: &str,
    max_bytes: usize,
) -> Result<PathBuf, String> {
    let dir = store.dir_scoped(workspace, scope);
    let slug = slugify(name);
    let path = dir.join(format!("{slug}.md"));
    // Cross-process lock: append is a read-modify-write (read existing content,
    // merge new facts, write back). The in-process WRITE_LOCK serializes
    // threads/subagents but NOT separate processes — two processes appending
    // to the same memory name concurrently would both read the same base and
    // the second rename would silently drop the first's facts. This advisory
    // flock (auto-released on exit/crash) closes that gap.
    let _lock = crate::fsutil::FileLock::acquire(&dir.join(".lock"))
        .map_err(|e| format!("failed to acquire memory lock: {e}"))?;
    let existing = parse_memory_file(&path);
    let mut combined = match &existing {
        Some(m) if !m.content.is_empty() => {
            let mut s = m.content.clone();
            if !s.ends_with('\n') {
                s.push('\n');
            }
            s.push_str("\n--- appended ---\n");
            s.push_str(new_facts);
            s
        }
        _ => new_facts.to_string(),
    };
    if combined.len() > max_bytes {
        // Keep the newest facts (the tail, since we append) and trim the oldest
        // from the front. We keep the last `max_bytes` verbatim; a mid-line start
        // is acceptable for a rolling fact buffer (a giant single-line fact must
        // not be dropped entirely just because it has no newline to snap to).
        let mut start = combined.len() - max_bytes;
        while !combined.is_char_boundary(start) {
            start += 1;
        }
        combined = format!(
            "[older auto-extracted facts trimmed to fit]\n{}",
            &combined[start..]
        );
    }
    // Appending preserves the existing memory's type/description/importance;
    // the caller's values only apply when creating a NEW memory, so `append`
    // can never silently wipe a memory's metadata (the tool defaults
    // description="", type="note").
    let (final_type, final_desc, final_importance) = match &existing {
        Some(m) if !m.content.is_empty() => {
            (m.mem_type.as_str(), m.description.as_str(), m.importance)
        }
        _ => (mem_type, description, Importance::Normal),
    };
    store.save_scoped_with_importance(
        workspace,
        scope,
        name,
        &combined,
        final_type,
        final_desc,
        final_importance,
    )
}

/// Delete a memory by its slug/id (the filename stem) and rebuild the index.
/// Accepts either the slug (file stem) or the original memory `name` — slugify()
/// normalizes both to the same filename, so only the slug candidate is needed.
/// slugify() strips '/', '\', and '.' to '-', so the joined path can never
/// escape the memory dir (no path-traversal deletion via a crafted id).
pub fn forget_memory(workspace: &Path, id: &str) -> Result<(), String> {
    forget_memory_scoped(workspace, Scope::Workspace, id)
}

/// Like `forget_memory` but for a specific scope.
pub fn forget_memory_scoped(workspace: &Path, scope: Scope, id: &str) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("memory id must not be empty".to_string());
    }
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let store = Store::new(Store::default_root());
    let dir = store.dir_scoped(workspace, scope);
    let slug = slugify(id);
    let path = dir.join(format!("{}.md", slug));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("failed to remove memory: {e}"))?;
        rebuild_index(&dir, scope)?;
        drop(_guard);
        invalidate_scan_cache();
        Ok(())
    } else {
        Err(format!("no memory found with id/name '{id}'"))
    }
}

/// Forget a memory by searching both scopes (workspace first, then global).
/// Used when the caller doesn't know which scope a memory lives in. Each
/// scoped forget acquires WRITE_LOCK internally, so this is safe to call
/// without an outer lock.
pub fn forget_memory_any(workspace: &Path, id: &str) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("memory id must not be empty".to_string());
    }
    forget_memory_scoped(workspace, Scope::Workspace, id)
        .or_else(|_| forget_memory_scoped(workspace, Scope::Global, id))
}

/// Look up a memory by id (slug) or name in both scopes (workspace first).
pub fn get_memory(workspace: &Path, id: &str) -> Result<MemoryEntry, String> {
    get_memory_scoped(workspace, Scope::Workspace, id)
        .or_else(|_| get_memory_scoped(workspace, Scope::Global, id))
}

/// Look up a memory by id/name in a specific scope.
pub fn get_memory_scoped(workspace: &Path, scope: Scope, id: &str) -> Result<MemoryEntry, String> {
    let store = Store::new(Store::default_root());
    let dir = store.dir_scoped(workspace, scope);
    let slug = slugify(id);
    if slug.is_empty() {
        return Err("memory id/name must contain at least one alphanumeric character".into());
    }
    let path = dir.join(format!("{slug}.md"));
    if !path.exists() {
        // Fall back to scanning by display name (slug may differ from id input).
        if let Some(entry) = store
            .scan_scoped(workspace, scope)
            .into_iter()
            .find(|e| e.name.eq_ignore_ascii_case(id.trim()) || slugify(&e.name) == slug)
        {
            return Ok(entry);
        }
        return Err(format!(
            "no {} memory found with id/name '{id}'",
            scope.as_str()
        ));
    }
    match parse_memory_file(&path) {
        Some(mut entry) => {
            entry.scope = scope;
            Ok(entry)
        }
        None => Err(format!("memory file at {} is unreadable", path.display())),
    }
}

/// True when a memory with this name/id already exists in the given scope.
pub fn memory_exists_scoped(workspace: &Path, scope: Scope, name: &str) -> bool {
    get_memory_scoped(workspace, scope, name).is_ok()
}

/// Count of memories across both scopes (for save-path soft warnings).
pub fn memory_count(workspace: &Path) -> usize {
    scan_all_memories(workspace).len()
}

/// Report from a stale-reference migration pass ([`migrate_memories`]).
#[derive(Clone, Debug, Default)]
pub struct MigrateReport {
    pub migrated: Vec<String>,
    pub message: String,
}

/// Old → new project-name substitution map applied by [`migrate_memories`].
/// Targets dead path/env references left by the umans-harness → catalyst-code
/// rename. The provider name "Umans" is intentionally NOT rewritten (it is a
/// distinct, still-valid name).
fn apply_rename_map(s: &str) -> String {
    s.replace("UMANS_CORE", "CATALYST_CODE")
        .replace(".umans-harness", ".catalyst-code")
        .replace("umans-harness", "catalyst-code")
}

/// Emit a memory markdown file from explicit parsed fields (preserving
/// `pinned`/`importance`/deprecation metadata exactly as parsed — unlike
/// [`Store::save_scoped_with_importance`], which re-derives `pin` from type).
fn write_memory_file(
    path: &Path,
    e: &MemoryEntry,
    content: &str,
    description: &str,
) -> std::io::Result<()> {
    let pin_line = if e.pinned { "pin: true\n" } else { "" };
    let importance_line = if e.importance != Importance::Normal {
        format!("importance: {}\n", e.importance.as_str())
    } else {
        String::new()
    };
    let dep_line = if e.deprecated {
        "deprecated: true\n".to_string()
    } else {
        String::new()
    };
    let sup_line = match &e.superseded_by {
        Some(s) if !s.trim().is_empty() => format!("superseded_by: {}\n", s),
        _ => String::new(),
    };
    let body = format!(
        "---\nname: {}\ndescription: {}\ntype: {}\n{pin_line}{importance_line}{dep_line}{sup_line}---\n{}",
        e.name, description, e.mem_type, content
    );
    atomic_write(path, &body)
}

/// One-time, idempotent migration of stale project-name references in memory
/// bodies + descriptions (`umans-harness` → `catalyst-code`, `UMANS_CORE` →
/// `CATALYST_CODE`). Architecture/convention docs that still point at dead
/// `.umans-harness/` / `UMANS_CORE` paths are actively misleading; this
/// rewrites them in place, preserving all metadata. Memories whose NAME
/// describes the rename itself are skipped so the historical record isn't
/// corrupted ("renamed umans-harness to …").
pub fn migrate_memories(workspace: &Path) -> Result<MigrateReport, String> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let entries = scan_all_memories(workspace);
    let mut report = MigrateReport::default();
    for e in &entries {
        let lname = e.name.to_lowercase();
        if lname.contains("rename") || lname.contains("naming") || lname.contains("migrat") {
            continue;
        }
        let new_content = apply_rename_map(&e.content);
        let new_desc = apply_rename_map(&e.description);
        if new_content == e.content && new_desc == e.description {
            continue;
        }
        write_memory_file(&e.path, e, &new_content, &new_desc)
            .map_err(|err| format!("failed to rewrite memory '{}': {err}", e.name))?;
        report.migrated.push(e.name.clone());
    }
    report.message = if report.migrated.is_empty() {
        "migrate: no stale references found".into()
    } else {
        format!(
            "migrate: rewrote {} memor(y/ies): {}",
            report.migrated.len(),
            report.migrated.join(", ")
        )
    };
    drop(_guard);
    if !report.migrated.is_empty() {
        invalidate_scan_cache();
    }
    Ok(report)
}

/// Mark a memory deprecated (superseded/invalidated) by rewriting its
/// frontmatter to set `deprecated: true` and (optionally) `superseded_by:
/// <name>`, preserving the body. Deprecated memories are excluded from the
/// standing catalog and the per-turn relevant tail; they remain visible via
/// `list`/`get`/`forget` so they can be audited. This is the invalidation
/// mechanism behind `memory save ... replaces=...` / `memory deprecate`.
pub fn mark_memory_deprecated(
    workspace: &Path,
    scope: Scope,
    id: &str,
    superseded_by: Option<&str>,
) -> Result<(), String> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let store = Store::new(Store::default_root());
    let dir = store.dir_scoped(workspace, scope);
    let slug = slugify(id);
    if slug.is_empty() {
        return Err("memory id/name must contain at least one alphanumeric character".into());
    }
    let path = dir.join(format!("{slug}.md"));
    if !path.exists() {
        return Err(format!(
            "no {} memory found with id/name '{id}'",
            scope.as_str()
        ));
    }
    let entry = parse_memory_file(&path).ok_or_else(|| format!("memory '{id}' is unreadable"))?;
    let mut new_entry = entry.clone();
    new_entry.deprecated = true;
    new_entry.superseded_by = superseded_by
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    write_memory_file(
        &path,
        &new_entry,
        &new_entry.content,
        &new_entry.description,
    )
    .map_err(|e| format!("failed to write memory: {e}"))?;
    rebuild_index(&dir, scope)?;
    drop(_guard);
    invalidate_scan_cache();
    Ok(())
}

/// Mark a memory deprecated, searching both scopes (workspace first). Used by
/// `memory save replaces=...` / `memory deprecate` when the caller doesn't know
/// which scope the superseded memory lives in.
pub fn mark_memory_deprecated_any(
    workspace: &Path,
    id: &str,
    superseded_by: Option<&str>,
) -> Result<(), String> {
    mark_memory_deprecated(workspace, Scope::Workspace, id, superseded_by)
        .or_else(|_| mark_memory_deprecated(workspace, Scope::Global, id, superseded_by))
}

/// One-line description for catalog display: prefer frontmatter description,
/// else the first non-empty content line. Truncated for standing-prompt budget.
fn catalog_blurb(entry: &MemoryEntry) -> String {
    let raw = if !entry.description.trim().is_empty() {
        entry.description.trim().to_string()
    } else {
        entry
            .content
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("")
            .to_string()
    };
    truncate_chars(&raw, CATALOG_DESC_MAX_CHARS)
}

fn truncate_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max).collect();
    format!("{truncated}…")
}

/// Build a string to inject into the system prompt with memories relevant to
/// the user's current prompt.
///
/// - Empty `prompt` (standing system prompt): a **catalog** of name/type/scope
///   + one-line description only — no body previews. Capped by entry count and
///   char budget; pinned memories are preferred when truncating.
/// - Non-empty `prompt` (per-turn relevance): matching memories with short body
///   previews, capped at [`RELEVANT_MAX_ENTRIES`]. Prefer
///   [`relevant_memories_tail`] for the transient turn-tail path.
pub fn memory_injection(workspace: &Path, prompt: &str) -> String {
    let memories = scan_all_memories(workspace);
    build_injection(&memories, prompt)
}

/// Transient per-turn relevant-memory block for the request tail (not persisted,
/// not spliced into the standing system prompt — keeps the prefix cache stable).
///
/// Cached for the lifetime of a user-prompt string within a process; mid-turn
/// model rounds reuse the same tail until a memory write invalidates the scan.
pub fn relevant_memories_tail(workspace: &Path, prompt: &str) -> String {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return String::new();
    }
    let hash = hash_workspace(workspace);
    {
        let cache = SCAN_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if cache.ws_hash == hash {
            if let Some((ref p, ref tail)) = cache.relevant {
                if p == prompt {
                    return tail.clone();
                }
            }
        }
    }
    let memories = scan_all_memories(workspace);
    crate::memory_recall::begin_turn(workspace, prompt, &memories);
    let tail = build_relevant_tail(&memories, prompt);
    {
        let mut cache = SCAN_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        // Only retain if scan still matches this workspace (a concurrent write
        // could have invalidated between scan and here).
        if cache.ws_hash == hash || cache.scanned_at.is_some() {
            cache.ws_hash = hash;
            cache.relevant = Some((prompt.to_string(), tail.clone()));
        }
    }
    tail
}

/// Per-turn relevant-memory tail WITHOUT recall telemetry (for subagents, whose
/// task is not a user turn and would otherwise clobber the orchestrator's
/// in-flight turn tracking in [`memory_recall::begin_turn`]). Returns the same
/// semantic `[RELEVANT MEMORIES]` block as [`relevant_memories_tail`].
pub fn relevant_tail_for_subagent(workspace: &Path, prompt: &str) -> String {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return String::new();
    }
    let memories = scan_all_memories(workspace);
    build_relevant_tail(&memories, prompt)
}

fn importance_rank(i: Importance) -> u8 {
    match i {
        Importance::High => 2,
        Importance::Normal => 1,
        Importance::Low => 0,
    }
}

fn build_injection(memories: &[MemoryEntry], prompt: &str) -> String {
    if memories.is_empty() {
        return String::new();
    }
    if prompt.is_empty() {
        return build_catalog(memories);
    }
    // Keyword-filtered with short previews (also used by relevant_memories_tail).
    build_relevant_tail(memories, prompt)
}

fn build_catalog(memories: &[MemoryEntry]) -> String {
    // Exclude deprecated (superseded) memories — the successor carries the
    // knowledge; they remain visible via `list`/`get`. Pinned/high-importance
    // first (stable within group by name), then the rest.
    let mut order: Vec<&MemoryEntry> = memories.iter().filter(|m| !m.deprecated).collect();
    order.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| importance_rank(b.importance).cmp(&importance_rank(a.importance)))
            .then_with(|| a.scope.as_str().cmp(b.scope.as_str())) // global before workspace
            .then_with(|| a.name.cmp(&b.name))
    });

    let mut out = String::from(
        "[MEMORY CATALOG] — name/type/scope + one-line summary only. \
         Full text: memory action=get with id/name. Prefer append over new saves. \
         Use consolidate to merge near-duplicates; skip trivia.\n",
    );
    let mut listed = 0usize;
    let mut omitted = 0usize;
    let mut pins_listed = 0usize;
    for m in order {
        // Cap pinned entries so a large set of pinned convention/decision
        // memories can't crowd out operational architecture/note/gotcha
        // knowledge. Over-budget pinned entries are omitted (still visible via
        // `list`/`get`); unpinned entries fill the rest on importance/scope/name.
        if m.pinned && pins_listed >= CATALOG_PIN_BUDGET {
            omitted += 1;
            continue;
        }
        let blurb = catalog_blurb(m);
        let line = format!(
            "- **{}** ({}, {}){}\n",
            m.name,
            if m.mem_type.is_empty() {
                "note"
            } else {
                m.mem_type.as_str()
            },
            m.scope.as_str(),
            if blurb.is_empty() {
                String::new()
            } else {
                format!(": {blurb}")
            }
        );
        if listed >= CATALOG_MAX_ENTRIES || out.len() + line.len() > CATALOG_MAX_CHARS {
            omitted += 1;
            continue;
        }
        out.push_str(&line);
        listed += 1;
        if m.pinned {
            pins_listed += 1;
        }
    }
    if omitted > 0 {
        out.push_str(&format!(
            "- …and {omitted} more (memory action=list, then get by id)\n"
        ));
    }
    out
}

fn build_relevant_tail(memories: &[MemoryEntry], prompt: &str) -> String {
    // Exclude deprecated (superseded) memories — the successor carries the
    // knowledge; they remain visible via `list`/`get`.
    let live: Vec<&MemoryEntry> = memories.iter().filter(|m| !m.deprecated).collect();
    if live.is_empty() {
        return String::new();
    }
    // Always-on semantic retrieval: tf·idf-weighted cosine over significant
    // tokens, plus a keyword bonus for exact name/description token hits.
    // This is the local Milestone-4 stand-in (no external embedding model): it
    // ranks by query relevance rather than type-pinning, so pinned-but-irrelevant
    // memories can no longer crowd out real matches. The synonym-miss signal
    // from `memory_recall` (body matched but name didn't) is what unfroze this —
    // the deferral gate's condition has been met.
    let idf = compute_idf(&live);
    let q = tfidf_vector(prompt, &idf);
    let mut scored: Vec<(&MemoryEntry, f64)> = live
        .iter()
        .copied()
        .filter_map(|m| {
            let text = format!("{} {} {}", m.name, m.description, m.content);
            let sem = cosine_sim(&q, &tfidf_vector(&text, &idf));
            // Exact name/description keyword hit is a strong signal — give it a
            // small flat bonus so a genuine match edges out a near-synonym, and
            // guarantees real matches surface even when the corpus is tiny.
            let kw = if is_name_relevant(m, prompt) {
                0.15
            } else {
                0.0
            };
            let score = sem + kw;
            if sem > 0.0 || kw > 0.0 {
                Some((m, score))
            } else {
                None
            }
        })
        .collect();
    if scored.is_empty() {
        return String::new();
    }
    // Relevance dominates; pinning/importance are only tie-breakers (a query-" "
    // relevant unpinned memory must outrank a pinned-but-irrelevant one).
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| importance_rank(b.0.importance).cmp(&importance_rank(a.0.importance)))
            .then_with(|| b.0.pinned.cmp(&a.0.pinned))
            .then_with(|| a.0.name.cmp(&b.0.name))
    });
    let total = scored.len();
    scored.truncate(RELEVANT_MAX_ENTRIES);

    let mut out = String::from(
        "[RELEVANT MEMORIES] — semantic matches for this turn (transient; \
         tf·idf cosine + keyword over the memory store). Use memory action=get for full text.\n",
    );
    for (m, _score) in &scored {
        let blurb = catalog_blurb(m);
        out.push_str(&format!(
            "- **{}** ({}, {}){}\n",
            m.name,
            if m.mem_type.is_empty() {
                "note"
            } else {
                m.mem_type.as_str()
            },
            m.scope.as_str(),
            if blurb.is_empty() {
                String::new()
            } else {
                format!(": {blurb}")
            }
        ));
        if !m.content.is_empty() {
            let preview: String = m
                .content
                .lines()
                .take(RELEVANT_PREVIEW_LINES)
                .collect::<Vec<_>>()
                .join("\n");
            out.push_str(&format!("  {preview}\n"));
        }
    }
    if total > scored.len() {
        out.push_str(&format!(
            "- …and {} more matches (memory action=list)\n",
            total - scored.len()
        ));
    }
    out
}

/// Document frequency of each significant token across the live memory corpus,
/// for idf weighting (rarer terms discriminate better; common tokens like
/// "core"/"system"/"file" get down-weighted so they can't false-match).
fn compute_idf(memories: &[&MemoryEntry]) -> std::collections::HashMap<String, f64> {
    let n = memories.len().max(1) as f64;
    let mut df: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for m in memories {
        let toks: std::collections::HashSet<String> =
            significant_tokens(&format!("{} {} {}", m.name, m.description, m.content))
                .into_iter()
                .collect();
        for t in toks {
            *df.entry(t).or_insert(0) += 1;
        }
    }
    df.into_iter()
        .map(|(t, d)| (t, (n / d.max(1) as f64).ln().max(0.0)))
        .collect()
}

/// tf·idf-weighted bag over significant tokens (local semantic vector).
fn tfidf_vector(
    text: &str,
    idf: &std::collections::HashMap<String, f64>,
) -> std::collections::HashMap<String, f64> {
    let mut v: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for t in significant_tokens(text) {
        let w = idf.get(&t).copied().unwrap_or(1.0);
        *v.entry(t).or_insert(0.0) += w;
    }
    v
}

pub fn cosine_sim(
    a: &std::collections::HashMap<String, f64>,
    b: &std::collections::HashMap<String, f64>,
) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for (k, va) in a {
        na += va * va;
        if let Some(vb) = b.get(k) {
            dot += va * vb;
        }
    }
    for vb in b.values() {
        nb += vb * vb;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

// ---- scan internals ----

fn scan_dir(dir: &Path, scope: Scope) -> Vec<MemoryEntry> {
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
        if let Some(mut entry) = parse_memory_file(&path) {
            entry.scope = scope;
            entries.push(entry);
        }
    }
    // Deterministic order (by name) so the injected memory block — and the
    // system prompt built from it — is stable across runs (prefix-cache safe).
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

// ---- frontmatter parser ----

/// Parse a memory markdown file. Returns None if the file can't be read or
/// has no valid frontmatter block (--- ... ---). Frontmatter fields are
/// simple `key: value` lines (YAML-like, hand-rolled). Everything after the
/// closing `---` is the content.
fn parse_memory_file(path: &Path) -> Option<MemoryEntry> {
    let raw = std::fs::read_to_string(path).ok()?;
    // Normalize CRLF -> LF up front. The byte-offset math below (slicing on
    // `end_pos`, `body_start`, and find_frontmatter_end's `line.len() + 1`)
    // all assume a single-byte '\n' terminator, but `str::lines()` strips a
    // trailing '\r' — so on CRLF files every line shifted the closing-fence
    // offset by one byte and silently corrupted the parsed fields and body.
    let raw = raw.replace("\r\n", "\n");
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---\n") {
        return None;
    }
    let after_open = &trimmed[3..];
    let after_open = after_open.strip_prefix('\n').unwrap_or(after_open);
    let end_pos = find_frontmatter_end(after_open)?;
    let fm_block = &after_open[..end_pos];
    let body_start = end_pos + 3;
    let rest = &after_open[body_start..];
    let content = rest.strip_prefix('\n').unwrap_or(rest).to_string();

    let mut name = String::new();
    let mut description = String::new();
    let mut mem_type = String::new();
    let mut pinned = false;
    let mut importance = Importance::Normal;
    let mut deprecated = false;
    let mut superseded_by: Option<String> = None;

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
            "importance" => importance = Importance::parse(&val),
            "pin" | "pinned" => {
                pinned = matches!(val.to_lowercase().as_str(), "true" | "yes" | "1");
            }
            "deprecated" => {
                deprecated = matches!(val.to_lowercase().as_str(), "true" | "yes" | "1");
            }
            "superseded_by" | "replaces" | "replaced_by" => {
                superseded_by = if val.trim().is_empty() {
                    None
                } else {
                    Some(val)
                };
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return None;
    }

    // Types that are almost always worth keeping in the standing catalog.
    if !pinned {
        pinned = is_pinned_type(&mem_type) || importance == Importance::High;
    }

    Some(MemoryEntry {
        name,
        description,
        mem_type,
        content,
        path: path.to_path_buf(),
        // The scope is determined by the directory the file lives in, not the
        // file content. scan_dir() overrides this with the correct value; the
        // default here covers direct parse_memory_file callers (e.g. append).
        scope: Scope::Workspace,
        pinned,
        importance,
        deprecated,
        superseded_by,
    })
}

/// Built-in pin heuristic for memory types that are almost always relevant
/// (identity-shaped) and so deserve a guaranteed catalog slot. Convention and
/// decision memories are NOT auto-pinned here — previously `is_pinned_type`
/// auto-pinned them, which let ~38 convention/decision memories seize ~38 of 48
/// catalog slots and crowd out operational architecture/note/gotcha knowledge.
/// Pin convention/decision explicitly via `pin: true` or `importance: high`
/// when a memory is truly always-relevant; otherwise let it compete on merit.
fn is_pinned_type(mem_type: &str) -> bool {
    matches!(
        mem_type.trim().to_lowercase().as_str(),
        "user" | "identity" | "preference"
    )
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
fn rebuild_index(dir: &Path, scope: Scope) -> Result<(), String> {
    let entries = scan_dir(dir, scope);
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

/// Public for recall telemetry: name+description keyword overlap with prompt.
pub fn is_name_relevant(entry: &MemoryEntry, prompt: &str) -> bool {
    let prompt_lower = prompt.to_lowercase();
    let text = format!("{} {}", entry.name, entry.description);
    significant_tokens(&text)
        .into_iter()
        .any(|kw| prompt_lower.contains(&kw))
}

/// Significant lowercase tokens (>2 chars, not stopwords) from `text`.
pub fn significant_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .map(|w| w.trim_matches(|c: char| c == '_' || c == '-'))
        .filter(|w| w.len() > 2)
        .filter(|w| !is_stopword(w))
        .map(|w| w.to_lowercase())
        .collect()
}

fn is_stopword(w: &str) -> bool {
    // Common, low-discrimination words excluded from significant tokens. The
    // bar is "would this false-match a keyword-relevance check?" — short common
    // words like "all"/"use"/"new"/"get" are the worst offenders: they appear in
    // most descriptions, so a user message containing them (e.g. "implement
    // all") matched nearly every memory and crowded out real matches. Code-
    // generic words (file/code/data) are similarly low-value in a coding-agent
    // store. The tf·idf cosine also down-weights these, but the keyword bonus
    // path uses raw token presence, so stopwords must cover them explicitly.
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
            | "were"
            | "has"
            | "had"
            | "have"
            | "not"
            | "but"
            | "its"
            | "can"
            | "all"
            | "any"
            | "new"
            | "use"
            | "used"
            | "using"
            | "get"
            | "set"
            | "put"
            | "add"
            | "via"
            | "etc"
            | "now"
            | "also"
            | "into"
            | "onto"
            | "over"
            | "under"
            | "more"
            | "most"
            | "one"
            | "two"
            | "file"
            | "code"
            | "data"
            | "here"
            | "when"
            | "then"
            | "than"
            | "will"
            | "would"
            | "could"
            | "should"
            | "may"
            | "might"
            | "must"
            | "you"
            | "your"
            | "they"
            | "them"
            | "their"
            | "what"
            | "which"
            | "how"
            | "why"
            | "where"
            | "some"
            | "such"
            | "very"
            | "much"
            | "many"
            | "each"
            | "every"
            | "both"
            | "only"
            | "even"
            | "still"
            | "yet"
            | "just"
            | "our"
            | "out"
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

/// Public wrapper so hygiene/recall can share the (possibly overridden) root.
pub fn memory_store_root_public() -> PathBuf {
    memory_store_root()
}

/// Public slug helper for recall/hygiene modules (same rules as internal slugify).
pub fn slugify_public(name: &str) -> String {
    slugify(name)
}

/// Atomic + fsync'd file write via a UNIQUE temp file (fsutil), so two
/// processes writing the same memory concurrently never collide on a shared
/// temp name and corrupt each other's write. Memories are durable learnings,
/// so they get the same crash-safety as session persistence (temp + fsync +
/// rename; an orphaned temp on crash is benign).
fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    crate::fsutil::atomic_write_str(path, content)
}

// ---- tests ----

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("catalyst_code_memory_test_{n}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn fake_workspace(name: &str) -> PathBuf {
        let ws = std::env::temp_dir().join(format!("catalyst_code_memory_ws_{name}"));
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
    fn scan_cache_reuses_until_write_invalidates() {
        let _serial = memory_test_serial()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let root = tmp_root();
        let ws = fake_workspace("scan_cache");
        let _guard = override_memory_root(root.clone());
        invalidate_scan_cache();
        save_memory_scoped(
            &ws,
            Scope::Workspace,
            "cache-fact",
            "body about widgets",
            "note",
            "widgets blurb",
        )
        .unwrap();
        let a = scan_all_memories(&ws);
        let b = scan_all_memories(&ws);
        assert_eq!(a.len(), b.len());
        assert_eq!(a[0].name, b[0].name);
        // Same prompt reuses the relevant-tail string.
        let t1 = relevant_memories_tail(&ws, "tell me about widgets");
        let t2 = relevant_memories_tail(&ws, "tell me about widgets");
        assert_eq!(t1, t2);
        assert!(t1.contains("cache-fact") || t1.contains("widgets"), "{t1}");
        // Write invalidates so a new name appears on next scan.
        save_memory_scoped(
            &ws,
            Scope::Workspace,
            "other-fact",
            "body about sprockets",
            "note",
            "sprockets blurb",
        )
        .unwrap();
        let c = scan_all_memories(&ws);
        assert_eq!(c.len(), 2);
        invalidate_scan_cache();
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&ws);
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
            scope: Scope::Workspace,
            pinned: true,
            importance: Importance::Normal,
            deprecated: false,
            superseded_by: None,
        };
        assert!(is_name_relevant(&e, "write a strict TypeScript component"));
        assert!(is_name_relevant(&e, "use enums in this file"));
        assert!(is_name_relevant(&e, "TypeScript rules"));
        assert!(!is_name_relevant(&e, "write a Python script"));
        assert!(!is_name_relevant(&e, "hello world"));
    }

    #[test]
    fn relevance_skips_stopwords() {
        let e = MemoryEntry {
            name: "formatting".into(),
            description: "the standard is to use 2-space indent and it has trailing commas".into(),
            mem_type: "project".into(),
            content: String::new(),
            path: PathBuf::from("/fake/fmt.md"),
            scope: Scope::Workspace,
            pinned: false,
            importance: Importance::Normal,
            deprecated: false,
            superseded_by: None,
        };
        assert!(!is_name_relevant(&e, "the quick brown fox"));
        assert!(!is_name_relevant(&e, "this and that"));
        assert!(is_name_relevant(&e, "use standard formatting please"));
        assert!(is_name_relevant(&e, "adjust indent width"));
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
        assert!(injection.contains("[RELEVANT MEMORIES]"));
        assert!(injection.contains("test rules"));
        assert!(injection.contains("Jest is the test framework"));
        assert!(!injection.contains("indent"));
        // Relevant tail may include a short body preview.
        assert!(injection.contains("run tests with jest"));
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
    fn memory_injection_empty_prompt_builds_catalog() {
        // Standing system prompt: catalog of all memories (name + one-line),
        // no multi-line body previews.
        let root = tmp_root();
        let ws = fake_workspace("empty");
        let store = test_store(&root);

        store
            .save(
                &ws,
                "rust rules",
                "no unsafe\nnever panic",
                "project",
                "safe Rust only",
            )
            .unwrap();
        store
            .save(&ws, "indent", "always use tabs", "user", "tab width 4")
            .unwrap();
        let memories = store.scan(&ws);
        let injection = build_injection(&memories, "");
        assert!(injection.contains("[MEMORY CATALOG]"));
        assert!(injection.contains("rust rules"));
        assert!(injection.contains("safe Rust only"));
        assert!(injection.contains("indent"));
        // Body lines beyond the one-line blurb must not appear.
        assert!(
            !injection.contains("never panic"),
            "catalog must not embed multi-line bodies: {injection}"
        );
    }

    #[test]
    fn memory_catalog_caps_entries_and_prefers_pinned() {
        let mut memories = Vec::new();
        for i in 0..(CATALOG_MAX_ENTRIES + 5) {
            memories.push(MemoryEntry {
                name: format!("note-{i:03}"),
                description: format!("desc {i}"),
                mem_type: "note".into(),
                content: "body".into(),
                path: PathBuf::from(format!("/fake/note-{i}.md")),
                scope: Scope::Workspace,
                pinned: false,
                importance: Importance::Normal,
                deprecated: false,
                superseded_by: None,
            });
        }
        memories.push(MemoryEntry {
            name: "zzz-pinned".into(),
            description: "must survive truncation".into(),
            mem_type: "convention".into(),
            content: "important".into(),
            path: PathBuf::from("/fake/pinned.md"),
            scope: Scope::Workspace,
            pinned: true,
            importance: Importance::High,
            deprecated: false,
            superseded_by: None,
        });
        let injection = build_catalog(&memories);
        assert!(injection.contains("[MEMORY CATALOG]"));
        assert!(
            injection.contains("zzz-pinned"),
            "pinned memory must be kept under budget: {injection}"
        );
        assert!(
            injection.contains("…and "),
            "overflow marker required: {injection}"
        );
        let listed = injection.lines().filter(|l| l.starts_with("- **")).count();
        assert_eq!(listed, CATALOG_MAX_ENTRIES);
    }

    #[test]
    fn stopword_all_no_longer_false_matches() {
        // Regression: "all" (3 chars, was NOT a stopword) matched every memory
        // whose description contained "all" against a user message containing
        // "all" (e.g. "implement all") — surfacing 5 pinned-but-irrelevant
        // architecture memories every turn. "all" is now a stopword.
        let e = MemoryEntry {
            name: "ship-policy".into(),
            description: "Repo .gitignore: only source + shipped; all build artifacts ignored"
                .into(),
            mem_type: "convention".into(),
            content: String::new(),
            path: PathBuf::from("/fake/g.md"),
            scope: Scope::Workspace,
            pinned: true,
            importance: Importance::High,
            deprecated: false,
            superseded_by: None,
        };
        assert!(
            !significant_tokens("implement all").contains(&"all".to_string()),
            "'all' must be a stopword"
        );
        assert!(
            !is_name_relevant(&e, "implement all"),
            "'all' false-match must not mark this memory relevant"
        );
    }

    #[test]
    fn semantic_tail_surfaces_real_match_not_common_token_noise() {
        // A pinned convention memory that only shares the common word "all" must
        // NOT crowd out an unpinned architecture memory that genuinely matches.
        let real = MemoryEntry {
            name: "self-learning-system".into(),
            description: "architecture of the self-learning memory system".into(),
            mem_type: "architecture".into(),
            content: "memory recall, skills, auto-reflect".into(),
            path: PathBuf::from("/fake/sl.md"),
            scope: Scope::Workspace,
            pinned: false,
            importance: Importance::Normal,
            deprecated: false,
            superseded_by: None,
        };
        let noise = MemoryEntry {
            name: "ship-policy".into(),
            description: "all build artifacts are ignored on ship".into(),
            mem_type: "convention".into(),
            content: "gitignore rules".into(),
            path: PathBuf::from("/fake/sp.md"),
            scope: Scope::Workspace,
            pinned: true,
            importance: Importance::High,
            deprecated: false,
            superseded_by: None,
        };
        let memories = vec![noise.clone(), real.clone()];
        let tail = build_injection(&memories, "what is our self learning system");
        assert!(
            tail.contains("self-learning-system"),
            "real match must surface: {tail}"
        );
        assert!(
            !tail.contains("ship-policy"),
            "common-token false match must NOT surface: {tail}"
        );
    }

    #[test]
    fn deprecated_memory_excluded_from_catalog_and_tail() {
        let live = MemoryEntry {
            name: "cursor-provider".into(),
            description: "current: native cursor provider via Connect-RPC".into(),
            mem_type: "architecture".into(),
            content: "the real facts".into(),
            path: PathBuf::from("/fake/live.md"),
            scope: Scope::Workspace,
            pinned: true,
            importance: Importance::High,
            deprecated: false,
            superseded_by: None,
        };
        let dead = MemoryEntry {
            name: "cursor-provider-old".into(),
            description: "stale: there is no native cursor provider".into(),
            mem_type: "architecture".into(),
            content: "wrong facts".into(),
            path: PathBuf::from("/fake/dead.md"),
            scope: Scope::Workspace,
            pinned: true,
            importance: Importance::High,
            deprecated: true,
            superseded_by: Some("cursor-provider".into()),
        };
        let catalog = build_catalog(&[live.clone(), dead.clone()]);
        assert!(catalog.contains("cursor-provider"));
        assert!(
            !catalog.contains("cursor-provider-old"),
            "deprecated must be excluded from catalog: {catalog}"
        );
        let tail = build_injection(&[live, dead], "cursor provider");
        assert!(
            !tail.contains("cursor-provider-old"),
            "deprecated must be excluded from relevant tail: {tail}"
        );
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
    fn concurrent_appends_do_not_lose_facts() {
        // Parallel subagents appending to the SAME memory name must not drop
        // facts: WRITE_LOCK serializes the read-modify-write. Without it two
        // threads reading the same content then both writing would lose one.
        let root = tmp_root();
        let ws = fake_workspace("concurrent");
        let store = test_store(&root);
        let n = 8usize;
        let mut handles = vec![];
        for i in 0..n {
            let store_root = root.clone();
            let ws2 = ws.clone();
            handles.push(std::thread::spawn(move || {
                let s = Store::new(store_root);
                append_memory_locked(
                    &s,
                    &ws2,
                    Scope::Workspace,
                    "shared",
                    &format!("fact-{i}"),
                    "note",
                    "desc",
                    1_000_000,
                )
                .unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let entries = store.scan(&ws);
        assert_eq!(entries.len(), 1, "one memory named 'shared'");
        let content = &entries[0].content;
        for i in 0..n {
            assert!(
                content.contains(&format!("fact-{i}")),
                "fact-{i} missing from appended memory: {content}"
            );
        }
    }

    #[test]
    fn slugify_produces_safe_filenames() {
        assert_eq!(slugify("user preferences"), "user-preferences");
        assert_eq!(slugify("TypeScript Rules!"), "typescript-rules");
        assert_eq!(slugify("  spaces  "), "spaces");
        assert_eq!(slugify("a/b:c"), "a-b-c");
        // traversal chars are stripped, so a crafted id can't escape the memory dir
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd");
        assert!(
            !slugify("../../etc/passwd").contains('/')
                && !slugify("../../etc/passwd").contains('.')
        );
    }

    #[test]
    fn append_preserves_existing_description_and_type() {
        // Appending must NOT clobber an existing memory's description/type —
        // the memory tool defaults description="" type="note", so a naive append
        // would wipe the metadata. The caller's values only apply to NEW memories.
        let root = tmp_root();
        let ws = fake_workspace("preserve");
        let store = test_store(&root);
        store
            .save(&ws, "skill", "body", "convention", "How we do X")
            .unwrap();
        append_memory_locked(
            &store,
            &ws,
            Scope::Workspace,
            "skill",
            "more facts",
            "",
            "",
            1_000_000,
        )
        .unwrap();
        let entries = store.scan(&ws);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].description, "How we do X");
        assert_eq!(entries[0].mem_type, "convention");
        assert!(entries[0].content.contains("body"));
        assert!(entries[0].content.contains("more facts"));
    }

    #[test]
    fn save_rejects_empty_or_punctuation_only_name() {
        // slugify("") and slugify("!!!") both yield "" — must be rejected, not
        // written as a hidden ".md" file (which scan would silently skip).
        let root = tmp_root();
        let ws = fake_workspace("emptyslug");
        let store = test_store(&root);
        assert!(store.save(&ws, "", "x", "note", "d").is_err());
        assert!(store.save(&ws, "!!!", "x", "note", "d").is_err());
        assert!(store.scan(&ws).is_empty());
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
    fn frontmatter_parses_crlf_line_endings() {
        // A memory file edited on Windows (CRLF) must parse identically to its
        // LF counterpart. Previously find_frontmatter_end's `line.len() + 1`
        // assumed a single-byte terminator while `str::lines()` strips the
        // trailing carriage return, so the closing-fence offset drifted and
        // corrupted the body/fields.
        let root = tmp_root();
        let ws = fake_workspace("crlf");
        let store = test_store(&root);
        let dir = store.dir(&ws);
        std::fs::create_dir_all(&dir).unwrap();

        let md = "---\r\nname: win config\r\ndescription: crlf desc\r\ntype: user\r\n---\r\nHere is the body.\r\nMultiline.\r\n";
        std::fs::write(dir.join("win.md"), md).unwrap();

        let entries = store.scan(&ws);
        assert_eq!(entries.len(), 1, "CRLF file must still be discovered");
        assert_eq!(entries[0].name, "win config");
        assert_eq!(entries[0].description, "crlf desc");
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

    #[test]
    fn append_memory_accumulates_and_caps() {
        let root = tmp_root();
        let ws = fake_workspace("append");
        let store = test_store(&root);
        // first append: no existing file -> writes new facts
        let _ = append_memory_into(
            &store,
            &ws,
            Scope::Workspace,
            "facts",
            "fact A",
            "note",
            "d",
            4096,
        )
        .unwrap();
        let entries = store.scan(&ws);
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].content.contains("fact A"),
            "{}",
            entries[0].content
        );

        // second append: accumulates onto the first (does NOT overwrite)
        let _ = append_memory_into(
            &store,
            &ws,
            Scope::Workspace,
            "facts",
            "fact B",
            "note",
            "d",
            4096,
        )
        .unwrap();
        let entries = store.scan(&ws);
        assert_eq!(entries.len(), 1);
        let c = &entries[0].content;
        assert!(c.contains("fact A"), "early fact must survive: {c}");
        assert!(c.contains("fact B"), "new fact must be present: {c}");

        // third append exceeds the cap -> oldest facts trimmed, newest survive
        let _ = append_memory_into(
            &store,
            &ws,
            Scope::Workspace,
            "facts",
            &"new big fact ".repeat(400),
            "note",
            "d",
            4096,
        )
        .unwrap();
        let entries = store.scan(&ws);
        assert_eq!(entries.len(), 1);
        let c = &entries[0].content;
        assert!(c.contains("trimmed to fit"), "must note trimming: {c}");
        assert!(
            c.contains("new big fact"),
            "newest must survive trimming: {c}"
        );
        assert!(!c.contains("fact A"), "oldest should be trimmed away: {c}");
    }

    #[test]
    fn global_memories_stored_separately_from_workspace() {
        // Global memories live in a fixed `global/` dir, separate from the
        // workspace-hashed dir. They must not leak into workspace scans and vice
        // versa.
        let root = tmp_root();
        let ws = fake_workspace("global1");
        let store = test_store(&root);

        store
            .save_scoped(
                &ws,
                Scope::Workspace,
                "project-rule",
                "use rust",
                "project",
                "",
            )
            .unwrap();
        store
            .save_scoped(&ws, Scope::Global, "user-name", "Alice", "user", "")
            .unwrap();

        // Workspace scan sees only the workspace memory
        let ws_entries = store.scan(&ws);
        assert_eq!(ws_entries.len(), 1);
        assert_eq!(ws_entries[0].name, "project-rule");
        assert_eq!(ws_entries[0].scope, Scope::Workspace);

        // Global scan sees only the global memory
        let global_entries = store.scan_scoped(&ws, Scope::Global);
        assert_eq!(global_entries.len(), 1);
        assert_eq!(global_entries[0].name, "user-name");
        assert_eq!(global_entries[0].scope, Scope::Global);
    }

    #[test]
    fn scan_all_merges_global_then_workspace() {
        // scan_all_memories returns global entries first, then workspace entries,
        // each sorted by name. This ordering puts universal user-level facts
        // before project-specific ones in the system prompt.
        let root = tmp_root();
        let ws = fake_workspace("global2");
        let store = test_store(&root);

        store
            .save_scoped(&ws, Scope::Workspace, "zzz-project", "p", "project", "")
            .unwrap();
        store
            .save_scoped(&ws, Scope::Global, "aaa-global", "g", "user", "")
            .unwrap();
        store
            .save_scoped(&ws, Scope::Workspace, "aaa-project", "p2", "project", "")
            .unwrap();
        store
            .save_scoped(&ws, Scope::Global, "zzz-global", "g2", "user", "")
            .unwrap();

        let all = scan_all_memories_with_store(&store, &ws);
        assert_eq!(all.len(), 4);
        // Global first (sorted by name), then workspace (sorted by name)
        assert_eq!(all[0].name, "aaa-global");
        assert_eq!(all[0].scope, Scope::Global);
        assert_eq!(all[1].name, "zzz-global");
        assert_eq!(all[1].scope, Scope::Global);
        assert_eq!(all[2].name, "aaa-project");
        assert_eq!(all[2].scope, Scope::Workspace);
        assert_eq!(all[3].name, "zzz-project");
        assert_eq!(all[3].scope, Scope::Workspace);
    }

    #[test]
    fn global_memories_persist_across_workspaces() {
        // The key property: a global memory saved from one workspace is visible
        // from a DIFFERENT workspace's scan_all_memories. This is what makes
        // them "cross-codebase".
        let root = tmp_root();
        let ws1 = fake_workspace("global3a");
        let ws2 = fake_workspace("global3b");
        let store = test_store(&root);

        // Save a global memory from ws1
        store
            .save_scoped(
                &ws1,
                Scope::Global,
                "user-prefs",
                "likes Rust + Go",
                "user",
                "",
            )
            .unwrap();

        // It must be visible from ws2 (a completely different workspace)
        let all = scan_all_memories_with_store(&store, &ws2);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "user-prefs");
        assert_eq!(all[0].scope, Scope::Global);
        assert_eq!(all[0].content, "likes Rust + Go");
    }

    #[test]
    fn forget_memory_any_searches_both_scopes() {
        // forget_memory_any finds a memory regardless of which scope it's in.
        let root = tmp_root();
        let ws = fake_workspace("global4");
        let store = test_store(&root);

        store
            .save_scoped(&ws, Scope::Global, "global-fact", "g", "user", "")
            .unwrap();
        store
            .save_scoped(&ws, Scope::Workspace, "ws-fact", "w", "project", "")
            .unwrap();

        // Forget the global one via forget_memory_any (no scope specified)
        forget_memory_any_with_store(&store, &ws, "global-fact").unwrap();
        let all = scan_all_memories_with_store(&store, &ws);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "ws-fact");

        // Forget the workspace one too
        forget_memory_any_with_store(&store, &ws, "ws-fact").unwrap();
        assert!(scan_all_memories_with_store(&store, &ws).is_empty());
    }

    #[test]
    fn memory_injection_includes_global_memories() {
        // The system-prompt injection must include global memories so the model
        // always carries forward cross-codebase user-level facts.
        let root = tmp_root();
        let ws = fake_workspace("global5");
        let store = test_store(&root);

        store
            .save_scoped(
                &ws,
                Scope::Global,
                "user-identity",
                "User is Alice",
                "user",
                "",
            )
            .unwrap();
        store
            .save_scoped(
                &ws,
                Scope::Workspace,
                "project-rule",
                "use tabs",
                "project",
                "",
            )
            .unwrap();

        let memories = scan_all_memories_with_store(&store, &ws);
        let injection = build_injection(&memories, "");
        assert!(injection.contains("[MEMORY CATALOG]"));
        assert!(injection.contains("user-identity"));
        // Description empty → catalog blurb falls back to first content line.
        assert!(injection.contains("User is Alice"));
        assert!(injection.contains("project-rule"));
        // scope tags present
        assert!(injection.contains("global"));
        assert!(injection.contains("workspace"));
        // Catalog must not embed multi-line body blocks (single-line blurb only).
        assert!(
            !injection.contains("\n  User is Alice"),
            "catalog should not indent body previews: {injection}"
        );
    }

    // Helper: scan_all_memories against a test store (not the default root).
    fn scan_all_memories_with_store(store: &Store, workspace: &Path) -> Vec<MemoryEntry> {
        let mut entries = store.scan_scoped(workspace, Scope::Global);
        entries.extend(store.scan_scoped(workspace, Scope::Workspace));
        entries
    }

    // Helper: forget_memory_any against a test store.
    fn forget_memory_any_with_store(
        store: &Store,
        workspace: &Path,
        id: &str,
    ) -> Result<(), String> {
        // Try workspace first, then global — mirrors the public forget_memory_any.
        let try_scope = |scope: Scope| -> Result<(), String> {
            let dir = store.dir_scoped(workspace, scope);
            let slug = slugify(id);
            let path = dir.join(format!("{slug}.md"));
            if path.exists() {
                std::fs::remove_file(&path).map_err(|e| format!("failed to remove memory: {e}"))?;
                rebuild_index(&dir, scope)?;
                Ok(())
            } else {
                Err(format!("no memory found with id/name '{id}'"))
            }
        };
        try_scope(Scope::Workspace).or_else(|_| try_scope(Scope::Global))
    }
}
