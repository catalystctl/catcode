//! Per-workspace recurrence log for the auto-reflect self-learning loop.
//!
//! One JSON record is appended per non-trivial completed turn recording the
//! "shape" of work done (tool sequence + file categories touched). On the next
//! reflect continuation, recurring shapes (seen ≥ 2×) are surfaced to the model
//! so it can decide whether to write a reusable skill — turning the "solve the
//! same shape ≥ 2×" rule (which the model cannot otherwise track across
//! sessions) into an evaluable signal.
//!
//! Storage: `~/.config/catalyst-code/patterns/<workspace-hash>.jsonl`, capped
//! at [`MAX_ENTRIES`] lines (oldest trimmed) so it stays bounded over time.
//! Single-writer by construction: only the main orchestrator turn appends, and
//! `start_turn` enforces one turn at a time — so no cross-turn write races and
//! no lock is needed (unlike the memory store, which parallel subagents share).
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Cap on retained entries. ~200 turns of history is plenty to detect
/// recurrence; older shapes are unlikely to recur and are trimmed.
const MAX_ENTRIES: usize = 200;

/// Tools whose presence characterizes the *action* shape. Recon/read-only
/// tools (read_file, grep, glob, list_dir, diagnostics, fetch, memory, git_*)
/// are excluded — they describe exploration, not the kind of work done.
pub const SHAPE_TOOLS: &[&str] = &[
    "bash",
    "write_file",
    "edit",
    "patch",
    "bulk_write",
    "bulk_edit",
    "todo_write",
    "spawn",
    "subagent",
];

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
struct PatternEntry {
    sig: String,
    label: String,
    ts: u64,
}

/// A patterns store rooted at `root`. The default root is the user's
/// `~/.config/catalyst-code/patterns`; tests inject a temp root.
struct Store {
    root: PathBuf,
}

impl Store {
    fn default_root() -> PathBuf {
        crate::config::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/catalyst-code/patterns")
    }

    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Path for a given workspace: `<root>/<hash>.jsonl`. The hash reuses
    /// memory.rs's canonical-path hashing so the patterns file correlates with
    /// the memory store for the same workspace.
    fn path(&self, workspace: &Path) -> PathBuf {
        let hash = crate::memory::project_hash(&workspace.to_string_lossy());
        self.root.join(format!("{hash}.jsonl"))
    }

    fn append(&self, workspace: &Path, sig: &str, label: &str) {
        if sig.is_empty() {
            return;
        }
        let _ = std::fs::create_dir_all(&self.root);
        let path = self.path(workspace);
        // Cross-process lock: append is a read-modify-write (read all lines,
        // push, trim, write back). Two processes in the same workspace both
        // completing a turn would otherwise race and silently drop entries.
        // Advisory flock (auto-released on exit/crash).
        let _lock = crate::fsutil::FileLock::acquire(&path.with_extension("lock"));
        let mut lines = read_lines(&path);
        let entry = PatternEntry {
            sig: sig.to_string(),
            label: truncate(label.trim(), 120).to_string(),
            ts: now_secs(),
        };
        let line = serde_json::to_string(&entry).unwrap_or_default();
        lines.push(line);
        if lines.len() > MAX_ENTRIES {
            let drop = lines.len() - MAX_ENTRIES;
            lines.drain(0..drop);
        }
        let mut out = lines.join("\n");
        out.push('\n');
        let _ = write_atomic(&path, &out);
    }

    /// Recurring shapes: signatures seen ≥ 2 times, most-recurring first. Each
    /// entry is `(count, most_recent_label)` so the reflect prompt can name it.
    fn recurring(&self, workspace: &Path) -> Vec<(usize, String)> {
        let entries = read_entries(&self.path(workspace));
        let mut agg: HashMap<String, (usize, String)> = HashMap::new();
        for e in entries {
            let slot = agg.entry(e.sig).or_insert((0, e.label.clone()));
            slot.0 += 1;
            slot.1 = e.label; // keep the most recent label
        }
        let mut rec: Vec<(usize, String)> = agg
            .into_iter()
            .filter(|(_, (c, _))| *c >= 2)
            .map(|(_, (c, label))| (c, label))
            .collect();
        // Most-recurring first; ties broken alphabetically for stable output.
        rec.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        rec
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn truncate(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

fn read_lines(path: &Path) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => s
            .lines()
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn read_entries(path: &Path) -> Vec<PatternEntry> {
    read_lines(path)
        .into_iter()
        .filter_map(|l| serde_json::from_str::<PatternEntry>(&l).ok())
        .collect()
}

fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    // Unique-temp atomic write (fsutil): two processes in the same workspace
    // never share a temp file, so a concurrent append can't corrupt this one.
    crate::fsutil::atomic_write_str(path, content)
}

/// Build a shape signature from the tool names used (in order) and the file
/// categories touched. Recon tools are filtered out; duplicates are removed
/// (order-preserving for tools, sorted for files). Two turns with the same
/// signature did "the same shape of work" — the recurrence signal.
pub fn shape_signature(tools: &[String], files: &[String]) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut t: Vec<&str> = Vec::new();
    for n in tools {
        if SHAPE_TOOLS.contains(&n.as_str()) && seen.insert(n.clone()) {
            t.push(n.as_str());
        }
    }
    let mut f: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
    f.sort_unstable();
    f.dedup();
    format!("{}|{}", t.join("+"), f.join(","))
}

/// Derive a coarse file category from a path: up to two leading directory
/// components plus the extension (e.g. `core/src/main.rs` → `core/src/*.rs`,
/// `tui/render.go` → `tui/*.go`, `README.md` → `*.md`). This groups edits to
/// different files in the same area as the same shape.
pub fn file_category(path: &str) -> String {
    let p = Path::new(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
    let dirs: Vec<&str> = p
        .parent()
        .map(|par| {
            par.components()
                .filter_map(|c| c.as_os_str().to_str())
                .take(2)
                .collect()
        })
        .unwrap_or_default();
    let dir_s = dirs.join("/");
    match (dir_s.as_str(), ext) {
        ("", "") => "<root>".to_string(),
        ("", e) => format!("*.{e}"),
        (d, "") => d.to_string(),
        (d, e) => format!("{d}/*.{e}"),
    }
}

/// Append a shape record for a completed turn (default user root). Best-effort:
/// a write failure is silently dropped (auto-reflect is advisory; a missed
/// recurrence is not worth crashing a turn over).
pub fn append_pattern(workspace: &Path, sig: &str, label: &str) {
    Store::new(Store::default_root()).append(workspace, sig, label);
}

/// Recurring shapes for a workspace (default user root): `(count, label)` for
/// each signature seen ≥ 2 times, most-recurring first.
pub fn recurring_patterns(workspace: &Path) -> Vec<(usize, String)> {
    Store::new(Store::default_root()).recurring(workspace)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "pattern-log-test-{}-{}",
            std::process::id(),
            now_secs()
        ));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    fn fake_workspace(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "pattern-ws-{name}-{}-{}",
            std::process::id(),
            now_secs()
        ))
    }

    #[test]
    fn shape_signature_filters_recon_and_dedups() {
        let tools: Vec<String> = ["read_file", "edit", "bash", "edit", "grep"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let files: Vec<String> = ["core/src/main.rs", "core/src/config.rs"]
            .iter()
            .map(|s| file_category(s))
            .collect();
        // recon tools (read_file, grep) dropped; edit deduped; files sorted+deduped.
        assert_eq!(shape_signature(&tools, &files), "edit+bash|core/src/*.rs");
    }

    #[test]
    fn shape_signature_empty_when_only_recon() {
        let tools: Vec<String> = ["read_file", "grep", "glob"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(shape_signature(&tools, &[]), "|");
    }

    #[test]
    fn file_category_groups_by_area_and_ext() {
        assert_eq!(file_category("core/src/main.rs"), "core/src/*.rs");
        assert_eq!(file_category("tui/render.go"), "tui/*.go");
        assert_eq!(file_category("README.md"), "*.md");
        assert_eq!(file_category("a/b/c/d.rs"), "a/b/*.rs");
        assert_eq!(file_category("x.txt"), "*.txt");
    }

    #[test]
    fn file_category_root_and_no_ext() {
        assert_eq!(file_category("Makefile"), "<root>");
        assert_eq!(file_category("docs/guide"), "docs");
    }

    #[test]
    fn append_and_recurring_counts() {
        let root = tmp_root();
        let store = Store::new(root);
        let ws = fake_workspace("counts");
        let sig = "edit+bash|core/src/*.rs";
        store.append(&ws, sig, "add a core tool");
        store.append(&ws, sig, "add another core tool");
        store.append(&ws, "write_file|tui/*.go", "add a tui renderer");

        let rec = store.recurring(&ws);
        // Only the first shape recurs (2×).
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].0, 2);
        assert!(rec[0].1.contains("another core tool")); // most recent label
    }

    #[test]
    fn recurring_empty_when_no_repeats() {
        let root = tmp_root();
        let store = Store::new(root);
        let ws = fake_workspace("norepeat");
        store.append(&ws, "edit|a", "one");
        store.append(&ws, "bash|b", "two");
        assert!(store.recurring(&ws).is_empty());
    }

    #[test]
    fn cap_trims_oldest() {
        let root = tmp_root();
        let store = Store::new(root);
        let ws = fake_workspace("cap");
        for i in 0..(MAX_ENTRIES + 50) {
            store.append(&ws, &format!("sig{i}"), &format!("label{i}"));
        }
        let lines = read_lines(&store.path(&ws));
        assert_eq!(lines.len(), MAX_ENTRIES);
        // oldest (sig0..) trimmed; newest retained.
        let last = lines.last().unwrap();
        assert!(last.contains("sig249"));
    }

    #[test]
    fn empty_sig_is_ignored() {
        let root = tmp_root();
        let store = Store::new(root);
        let ws = fake_workspace("empty");
        store.append(&ws, "", "nothing");
        assert!(store.recurring(&ws).is_empty());
    }
}
