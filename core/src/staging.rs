//! First-run global staging.
//!
//! The harness ships a set of *default* subagent files — the built-in agent
//! definitions (`agents/*.md`), the orchestrator delegation skill
//! (`skills/pi-subagents/SKILL.md`), and the vision-handoff plugin. These are
//! the things every project needs for the agent system to work, and they
//! should NOT be copied into each project's `.catalyst-code/`. Instead they are
//! materialized once, at a single **global, user-owned** location —
//! `~/.catalyst-code/` — so they are shared across every project and editable in
//! one place.
//!
//! A project's own `.catalyst-code/` remains a deliberate **override**: any file
//! placed there shadows the global default for that project only. Nothing is
//! staged per-project by default.
//!
//! Staging is:
//! - **idempotent** — re-running only fills in files that are still missing;
//! - **non-clobbering** — a file the user edited (or deleted then recreated) is
//!   never overwritten;
//! - **versioned** — a marker file (`.staged`) records the staging schema so a
//!   bump can fill in newly-added defaults, while still leaving existing files
//!   untouched.

use crate::config::home_dir;
use std::io::Write;
use std::path::PathBuf;

/// Bump when the bundled default set changes meaningfully. The marker file
/// stores this; on a version mismatch we re-scan for *missing* files (existing
/// user files are still never overwritten) and then re-stamp the marker.
pub const STAGING_VERSION: u32 = 2;

/// `~/.catalyst-code` — the global, user-owned home for harness defaults.
/// All staged files live under here (agents/, skills/, plugins/, README.md).
pub fn global_home() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".catalyst-code")
}

/// Recursively copy a directory tree (files + subdirs). Symlinks and other
/// special types are skipped for safety (a migration never follows links).
/// A per-entry error aborts the copy; the caller cleans up the partial dest.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else if ft.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Move `src` -> `dst`. Tries an atomic rename first; on failure (typically a
/// cross-device EXDEV, e.g. `~/.config` on a separate mount from `$HOME`) falls
/// back to a recursive copy + removal of the source. If the copy fails, the
/// partial destination is removed so the next run retries cleanly — the source
/// is only deleted after a complete, successful copy.
fn move_dir(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_rename_err) => {
            if let Err(copy_err) = copy_dir_recursive(src, dst) {
                let _ = std::fs::remove_dir_all(dst);
                return Err(copy_err);
            }
            std::fs::remove_dir_all(src)?;
            Ok(())
        }
    }
}

/// One-time migration from the pre-rename on-disk layout to the current names.
///
/// Moves (preserving all contents):
/// - `~/.config/umans-harness/` -> `~/.config/catalyst-code/` — sessions,
///   memory, OAuth tokens, settings, telemetry, patterns, escalation sidecars.
/// - `~/.umans-harness/` -> `~/.catalyst-code/` — staged default agents/skills/
///   plugins + user customizations + the `.staged` version marker.
///
/// Idempotent and non-clobbering: if the destination already exists (newer
/// install, prior migration, or the user running both versions), the legacy dir
/// is left untouched rather than risk overwriting newer data. Safe to call on
/// every startup — a no-op once the new layout is present.
pub fn migrate_legacy_dirs() {
    if let Some(home) = home_dir() {
        migrate_legacy_dirs_in(&home);
    }
}

/// Testable core: migrate the legacy layout rooted at `home` (no `$HOME` env
/// dependency, so tests don't race with other env-mutating tests).
pub(super) fn migrate_legacy_dirs_in(home: &std::path::Path) {
    let pairs: [(std::path::PathBuf, std::path::PathBuf); 2] = [
        (
            home.join(".config").join("umans-harness"),
            home.join(".config").join("catalyst-code"),
        ),
        (home.join(".umans-harness"), home.join(".catalyst-code")),
    ];
    for (src, dst) in pairs {
        if dst.exists() || !src.exists() {
            continue;
        }
        if let Some(parent) = dst.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match move_dir(&src, &dst) {
            Ok(()) => eprintln!(
                "[catalyst-code] migrated {} -> {} (preserved your existing data)",
                src.display(),
                dst.display()
            ),
            Err(e) => eprintln!(
                "[catalyst-code] could not migrate {} -> {}: {}. Your existing data is \
                 untouched; move it manually if you want it at the new path.",
                src.display(),
                dst.display(),
                e
            ),
        }
    }
}

/// What `stage_if_needed` did, for logging / surfacing to the user.
#[derive(Debug, Default)]
pub struct StageResult {
    /// True the first time the harness runs (no `.staged` marker at the
    /// current version). Also true after a version bump.
    pub first_run: bool,
    /// Relative paths (e.g. `agents/scout.md`) written this run. Empty on a
    /// no-op run where everything was already present.
    pub written: Vec<String>,
    /// The global home that was staged into.
    pub home: PathBuf,
}

/// The bundled default files, as `(relative_path, content)` pairs. The content
/// is embedded into the core binary at build time via `include_str!`, so the
/// binary is self-sufficient — staging needs no external files on disk.
///
/// `include_str!` paths are relative to *this* source file (`core/src/`), so
/// `../../.catalyst-code/...` resolves to the repo-root `.catalyst-code/`,
/// which is the canonical source for these defaults.
fn bundled_files() -> Vec<(&'static str, &'static str)> {
    vec![
        // --- Built-in agent definitions (override templates for the 8
        // builtins; the binary also carries embedded fallback prompts, so
        // agents work even if these are deleted). ---
        (
            "agents/scout.md",
            include_str!("../../.catalyst-code/agents/scout.md"),
        ),
        (
            "agents/researcher.md",
            include_str!("../../.catalyst-code/agents/researcher.md"),
        ),
        (
            "agents/planner.md",
            include_str!("../../.catalyst-code/agents/planner.md"),
        ),
        (
            "agents/worker.md",
            include_str!("../../.catalyst-code/agents/worker.md"),
        ),
        (
            "agents/reviewer.md",
            include_str!("../../.catalyst-code/agents/reviewer.md"),
        ),
        (
            "agents/context-builder.md",
            include_str!("../../.catalyst-code/agents/context-builder.md"),
        ),
        (
            "agents/oracle.md",
            include_str!("../../.catalyst-code/agents/oracle.md"),
        ),
        (
            "agents/delegate.md",
            include_str!("../../.catalyst-code/agents/delegate.md"),
        ),
        // --- Orchestrator delegation skill (required for the parent agent to
        // know how to use the `subagent` tool + intercom). ---
        (
            "skills/pi-subagents/SKILL.md",
            include_str!("../../.catalyst-code/skills/pi-subagents/SKILL.md"),
        ),
        // --- vision-handoff plugin (required for image-bearing turns to route
        // to a vision-capable model). ---
        (
            "plugins/vision-handoff/plugin.json",
            include_str!("../../.catalyst-code/plugins/vision-handoff/plugin.json"),
        ),
        (
            "plugins/vision-handoff/hooks/pre_turn.py",
            include_str!("../../.catalyst-code/plugins/vision-handoff/hooks/pre_turn.py"),
        ),
        (
            "plugins/vision-handoff/README.md",
            include_str!("../../.catalyst-code/plugins/vision-handoff/README.md"),
        ),
        // --- telemetry plugin (aggregates per-turn metrics into a per-workspace
        // telemetry summary; fires on the session_stop lifecycle hook). ---
        (
            "plugins/telemetry/plugin.json",
            include_str!("../../.catalyst-code/plugins/telemetry/plugin.json"),
        ),
        (
            "plugins/telemetry/hooks/session_stop.py",
            include_str!("../../.catalyst-code/plugins/telemetry/hooks/session_stop.py"),
        ),
        (
            "plugins/telemetry/README.md",
            include_str!("../../.catalyst-code/plugins/telemetry/README.md"),
        ),
        // --- A short guide to the global layout + override model. ---
        ("README.md", GLOBAL_README),
    ]
}

/// Files that must be marked executable on Unix (hook scripts).
fn executable_rel_paths() -> &'static [&'static str] {
    &[
        "plugins/vision-handoff/hooks/pre_turn.py",
        "plugins/telemetry/hooks/session_stop.py",
    ]
}

/// Write `content` to `path` atomically: a sibling temp file is written, fsync'd,
/// then renamed over the target. On error the temp is removed, so a crash
/// mid-write can never leave a truncated file that the per-file `exists()`
/// short-circuit would then treat as complete (and never re-stage). Keeps the
/// idempotent/non-clobbering semantics — only the write durability changes.
fn atomic_write(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.flush()?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path).inspect_err(|_e| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Ensure the global default files exist under `~/.catalyst-code/`. Writes only
/// files that are missing; never overwrites. Returns what it did.
///
/// Safe to call on every startup: the per-file `exists()` check makes it a cheap
/// no-op once everything is staged. The `.staged` marker records the schema
/// version so a bump can backfill newly-added defaults.
pub fn stage_if_needed() -> StageResult {
    let home = global_home();
    // Best-effort: create the tree up front; individual file parents are also
    // created below, but this guarantees the top-level dir exists for the marker.
    let _ = std::fs::create_dir_all(&home);

    let marker = home.join(".staged");
    // Single source of truth: the marker stores the numeric staging version as
    // a string. A mismatch (missing marker, or an older version) means this is
    // a "first run" for messaging / backfill purposes.
    let version = STAGING_VERSION.to_string();
    let current_marker = std::fs::read_to_string(&marker).ok();
    let first_run = current_marker.as_deref() != Some(version.as_str());

    let mut written = Vec::new();
    for (rel, content) in bundled_files() {
        let path = home.join(rel);
        // Non-clobbering: skip anything already present (user-edited or not).
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if atomic_write(&path, content).is_ok() {
            written.push(rel.to_string());
            #[cfg(unix)]
            if executable_rel_paths().contains(&rel) {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
            }
        }
    }

    // Stamp the marker at the current version so subsequent runs short-circuit
    // `first_run` (we still re-scan for missing files above, but this keeps the
    // "is this the very first run" signal stable for user-facing messaging).
    let _ = atomic_write(&marker, &version);

    StageResult {
        first_run,
        written,
        home,
    }
}

/// Staged into `~/.catalyst-code/README.md` on first run.
const GLOBAL_README: &str = r#"# catalyst-code — global home

This directory (`~/.catalyst-code/`) is the **global, user-owned** home for the
harness's default agent files. It is shared across every project you run the
harness in, so defaults are configured once here — not copied into each
project.

## Layout

    ~/.catalyst-code/
    ├── agents/            # built-in subagent definitions (*.md)
    ├── skills/
    │   └── pi-subagents/  # orchestrator delegation skill (parent-only)
    ├── plugins/
    │   └── vision-handoff/# routes image turns to a vision-capable model
    ├── README.md          # this file
    └── .staged            # staging schema version marker (do not edit)

## Overrides

Defaults live here, globally. To override for a **single project**, place the
file under that project's own `.catalyst-code/` — it shadows the global one for
that project only. For example, to customize the `scout` agent in one project:

    <project>/.catalyst-code/agents/scout.md

Project files never modify the global defaults, and nothing is staged into a
project automatically.

## Editing the global defaults

Edit any file here to change a default for *every* project. The harness never
overwrites a file that already exists, so your edits are safe across upgrades;
delete a file to restore its bundled default on the next run.

## Agents vs. the embedded fallbacks

The eight agent definitions under `agents/` are override templates. The harness
binary also carries embedded fallback prompts, so agents keep working even if
you delete these files — they just lose your customizations.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_is_idempotent_and_nonclobbering() {
        let tmp = tempfile_dir();
        let home = tmp.join(".catalyst-code");
        std::env::set_var("HOME", &tmp);

        // First run: everything missing → all staged, first_run true.
        let r1 = stage_if_needed();
        assert!(r1.first_run, "first run should be marked first_run");
        assert!(!r1.written.is_empty(), "first run should write defaults");
        assert!(home.join("agents/scout.md").exists());
        assert!(home.join("skills/pi-subagents/SKILL.md").exists());
        assert!(home.join("plugins/vision-handoff/plugin.json").exists());
        assert!(home
            .join("plugins/vision-handoff/hooks/pre_turn.py")
            .exists());
        assert!(
            home.join("plugins/telemetry/plugin.json").exists(),
            "telemetry plugin should be staged on first run"
        );
        assert!(home
            .join("plugins/telemetry/hooks/session_stop.py")
            .exists());
        assert!(home.join(".staged").exists());
        assert_eq!(
            std::fs::read_to_string(home.join(".staged")).unwrap(),
            STAGING_VERSION.to_string(),
            "marker must record the current staging version"
        );

        // Second run: marker present → not first_run, nothing re-written.
        let r2 = stage_if_needed();
        assert!(!r2.first_run, "second run should not be first_run");
        assert!(r2.written.is_empty(), "second run should write nothing");

        // User edits are preserved (non-clobbering): mutate a file, re-stage,
        // confirm the edit survives.
        let scout = home.join("agents/scout.md");
        std::fs::write(&scout, "USER EDITED").unwrap();
        let r3 = stage_if_needed();
        assert_eq!(std::fs::read_to_string(&scout).unwrap(), "USER EDITED");
        assert!(
            !r3.written.contains(&"agents/scout.md".to_string()),
            "existing file must not be re-written"
        );

        // A deleted file is restored on the next run (backfill), while the
        // marker/first_run stays stable.
        let skill = home.join("skills/pi-subagents/SKILL.md");
        std::fs::remove_file(&skill).unwrap();
        let r4 = stage_if_needed();
        assert!(skill.exists(), "deleted default should be restored");
        assert!(r4
            .written
            .contains(&"skills/pi-subagents/SKILL.md".to_string()));
        assert!(!r4.first_run, "backfill is not a first_run");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(home.join("plugins/vision-handoff/hooks/pre_turn.py"))
                .unwrap()
                .permissions()
                .mode();
            assert!(mode & 0o111 != 0, "hook script must be executable");
            let mode = std::fs::metadata(home.join("plugins/telemetry/hooks/session_stop.py"))
                .unwrap()
                .permissions()
                .mode();
            assert!(
                mode & 0o111 != 0,
                "telemetry hook script must be executable"
            );
        }

        std::env::remove_var("HOME");
    }

    #[test]
    fn migrate_legacy_dirs_moves_old_layout() {
        let tmp = tempfile_dir();
        // Legacy layout with sentinel files under both old roots.
        let old_cfg = tmp.join(".config").join("umans-harness");
        std::fs::create_dir_all(old_cfg.join("sessions")).unwrap();
        std::fs::write(old_cfg.join("settings.json"), "{\"v\":1}").unwrap();
        let old_stage = tmp.join(".umans-harness");
        std::fs::create_dir_all(old_stage.join("agents")).unwrap();
        std::fs::write(old_stage.join(".staged"), "2").unwrap();
        std::fs::write(old_stage.join("agents/scout.md"), "USER CUSTOMIZED").unwrap();

        migrate_legacy_dirs_in(&tmp);

        // New layout holds the sentinel files; old roots are gone.
        assert!(tmp
            .join(".config")
            .join("catalyst-code")
            .join("settings.json")
            .exists());
        assert!(tmp.join(".catalyst-code").join(".staged").exists());
        assert_eq!(
            std::fs::read_to_string(tmp.join(".catalyst-code").join("agents/scout.md")).unwrap(),
            "USER CUSTOMIZED",
            "user customizations must be preserved across the migration"
        );
        assert!(!old_cfg.exists(), "old config root should be removed after migration");
        assert!(!old_stage.exists(), "old staging root should be removed after migration");

        // Idempotent: a second call is a no-op (new layout present -> skip).
        migrate_legacy_dirs_in(&tmp);
        assert!(tmp
            .join(".config")
            .join("catalyst-code")
            .join("settings.json")
            .exists());
    }

    /// A fresh temp dir to use as a fake $HOME for the staging test. Uses
    /// `std::env::temp_dir` + a unique subdir so tests don't collide.
    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "catalyst-code-staging-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
