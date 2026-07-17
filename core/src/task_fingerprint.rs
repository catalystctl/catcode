//! Semantic coding-task fingerprints (spec §7.3).
//!
//! Replaces tool-only recurrence signatures ([`crate::pattern_log`]) with a
//! richer, stable description of *what kind of coding work* a turn performed.
//! Matching uses set overlap — exact tool sequences are intentionally NOT the
//! primary signal so similar tasks remain recognizable across different
//! agent tool choices.

#![allow(dead_code)]
use serde::{Deserialize, Serialize};

/// Compact semantic fingerprint of a coding task.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskFingerprint {
    /// Coarse intent label, e.g. `extend-tool-schema`, `fix-bug`.
    #[serde(default)]
    pub intent: String,
    #[serde(default)]
    pub subsystems: Vec<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub frameworks: Vec<String>,
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub file_categories: Vec<String>,
    #[serde(default)]
    pub operations: Vec<String>,
    #[serde(default)]
    pub diagnostic_classes: Vec<String>,
    #[serde(default)]
    pub validation_classes: Vec<String>,
}

/// Inputs used to build a fingerprint from a completed (or in-progress) turn.
#[derive(Clone, Debug, Default)]
pub struct FingerprintInputs<'a> {
    pub user_intent: &'a str,
    pub files_read: &'a [String],
    pub files_changed: &'a [String],
    pub symbols: &'a [String],
    pub tools_used: &'a [String],
    pub diagnostics: &'a [String],
    pub tests_run: &'a [String],
}

/// Build a [`TaskFingerprint`] from turn evidence.
pub fn build_fingerprint(input: &FingerprintInputs<'_>) -> TaskFingerprint {
    let mut fp = TaskFingerprint {
        intent: infer_intent(input.user_intent, input.tools_used, input.files_changed),
        subsystems: infer_subsystems(input.files_read, input.files_changed),
        languages: infer_languages(input.files_read, input.files_changed),
        frameworks: Vec::new(),
        symbols: capped_unique(input.symbols, 24),
        file_categories: {
            let mut cats: Vec<String> = input
                .files_changed
                .iter()
                .chain(input.files_read.iter())
                .map(|p| crate::pattern_log::file_category(p))
                .collect();
            sort_dedup(&mut cats);
            cats.truncate(16);
            cats
        },
        operations: infer_operations(input.tools_used, input.files_changed),
        diagnostic_classes: capped_unique(input.diagnostics, 12),
        validation_classes: infer_validation(input.tests_run),
    };
    // Keep frameworks empty unless we later add lightweight detectors; do not
    // invent them from path heuristics alone.
    let _ = &mut fp.frameworks;
    fp
}

/// Similarity in `0.0..=1.0` using weighted Jaccard over fingerprint fields.
/// Deterministic and independent of tool-choice order.
pub fn fingerprint_similarity(a: &TaskFingerprint, b: &TaskFingerprint) -> f32 {
    let mut score = 0.0f32;
    let mut weight = 0.0f32;

    // Intent: exact match is strong; token overlap otherwise.
    weight += 0.25;
    score += 0.25 * intent_sim(&a.intent, &b.intent);

    weight += 0.20;
    score += 0.20 * jaccard(&a.symbols, &b.symbols);

    weight += 0.15;
    score += 0.15 * jaccard(&a.subsystems, &b.subsystems);

    weight += 0.10;
    score += 0.10 * jaccard(&a.file_categories, &b.file_categories);

    weight += 0.10;
    score += 0.10 * jaccard(&a.operations, &b.operations);

    weight += 0.08;
    score += 0.08 * jaccard(&a.languages, &b.languages);

    weight += 0.07;
    score += 0.07 * jaccard(&a.diagnostic_classes, &b.diagnostic_classes);

    weight += 0.05;
    score += 0.05 * jaccard(&a.validation_classes, &b.validation_classes);

    if weight <= 0.0 {
        0.0
    } else {
        (score / weight).clamp(0.0, 1.0)
    }
}

fn intent_sim(a: &str, b: &str) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let ta = tokenize(a);
    let tb = tokenize(b);
    jaccard(&ta, &tb)
}

fn jaccard(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let set_a: std::collections::HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let set_b: std::collections::HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    let inter = set_a.intersection(&set_b).count() as f32;
    let union = set_a.union(&set_b).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn tokenize(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|w| w.len() > 1)
        .map(|w| w.to_lowercase())
        .collect()
}

fn sort_dedup(v: &mut Vec<String>) {
    v.sort();
    v.dedup();
}

fn capped_unique(items: &[String], cap: usize) -> Vec<String> {
    let mut out: Vec<String> = items
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    sort_dedup(&mut out);
    out.truncate(cap);
    out
}

fn infer_intent(prompt: &str, tools: &[String], changed: &[String]) -> String {
    let p = prompt.to_lowercase();
    let has = |k: &str| p.contains(k);
    if has("add provider") || has("new provider") || has("key-auth") {
        return "add-provider".into();
    }
    if has("memory") && (has("action") || has("tool") || has("schema")) {
        return "extend-memory-tool".into();
    }
    if has("tool") && (has("schema") || has("action") || has("dispatch")) {
        return "extend-tool-schema".into();
    }
    if has("skill") {
        return "skill-work".into();
    }
    if has("plugin") {
        return "plugin-work".into();
    }
    if has("test") && (has("fix") || has("fail") || has("flaky")) {
        return "fix-test".into();
    }
    if has("refactor") {
        return "refactor".into();
    }
    if has("fix") || has("bug") || has("error") || has("panic") {
        return "fix-bug".into();
    }
    if has("document") || has("readme") || has("docs") {
        return "docs".into();
    }
    // Fallback from tools/files.
    let editish = tools.iter().any(|t| {
        matches!(
            t.as_str(),
            "edit" | "write_file" | "patch" | "bulk_edit" | "bulk_write"
        )
    });
    if editish && changed.iter().any(|f| f.contains("test")) {
        return "test-change".into();
    }
    if editish {
        return "code-change".into();
    }
    if !tools.is_empty() {
        return "explore".into();
    }
    "unknown".into()
}

fn infer_subsystems(read: &[String], changed: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for p in read.iter().chain(changed.iter()) {
        let lower = p.replace('\\', "/").to_lowercase();
        let top = lower
            .split('/')
            .find(|c| !c.is_empty() && *c != "." && *c != "..");
        if let Some(t) = top {
            let sub = match t {
                "core" => {
                    if lower.contains("memory") {
                        "memory"
                    } else if lower.contains("tool") {
                        "tools"
                    } else if lower.contains("provider") || lower.contains("oauth") {
                        "provider"
                    } else if lower.contains("plugin") {
                        "plugins"
                    } else if lower.contains("subagent") {
                        "subagent"
                    } else {
                        "core"
                    }
                }
                "tui" => "tui",
                "web" => "web",
                "sdk" => "sdk",
                "docs" => "docs",
                other => other,
            };
            out.push(sub.to_string());
        }
    }
    sort_dedup(&mut out);
    out.truncate(8);
    out
}

fn infer_languages(read: &[String], changed: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for p in read.iter().chain(changed.iter()) {
        if let Some(ext) = std::path::Path::new(p).extension().and_then(|e| e.to_str()) {
            let lang = match ext {
                "rs" => "rust",
                "go" => "go",
                "ts" | "tsx" => "typescript",
                "js" | "jsx" | "mjs" | "cjs" => "javascript",
                "py" => "python",
                "md" | "mdx" => "markdown",
                "json" => "json",
                "toml" => "toml",
                "yaml" | "yml" => "yaml",
                _ => continue,
            };
            out.push(lang.to_string());
        }
    }
    sort_dedup(&mut out);
    out
}

fn infer_operations(tools: &[String], changed: &[String]) -> Vec<String> {
    let mut ops = Vec::new();
    for t in tools {
        match t.as_str() {
            "edit" | "bulk_edit" | "patch" => ops.push("edit".into()),
            "write_file" | "bulk_write" => ops.push("create-file".into()),
            "bash" => ops.push("shell".into()),
            "subagent" | "spawn" => ops.push("delegate".into()),
            "todo_write" => ops.push("plan".into()),
            _ => {}
        }
    }
    for f in changed {
        let lower = f.to_lowercase();
        if lower.contains("test") {
            ops.push("test-change".into());
        }
        if lower.ends_with(".rs") && lower.contains("tool") {
            ops.push("dispatch-change".into());
            ops.push("schema-change".into());
        }
    }
    sort_dedup(&mut ops);
    ops.truncate(12);
    ops
}

fn infer_validation(tests: &[String]) -> Vec<String> {
    let mut out: Vec<String> = tests
        .iter()
        .map(|t| {
            let lower = t.to_lowercase();
            if lower.contains("cargo test") {
                // Keep a short class: `cargo-test-<filter>` when present.
                if let Some(rest) = lower.split("cargo test").nth(1) {
                    let filter = rest
                        .split_whitespace()
                        .find(|w| !w.starts_with('-'))
                        .unwrap_or("all");
                    format!("cargo-test-{filter}")
                } else {
                    "cargo-test".into()
                }
            } else if lower.contains("cargo build") || lower.contains("cargo check") {
                "cargo-build".into()
            } else if lower.contains("go test") {
                "go-test".into()
            } else if lower.contains("npm test") || lower.contains("pnpm test") {
                "js-test".into()
            } else {
                truncate(t, 48).to_string()
            }
        })
        .collect();
    sort_dedup(&mut out);
    out.truncate(12);
    out
}

fn truncate(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similar_tasks_match_despite_different_tools() {
        let a = build_fingerprint(&FingerprintInputs {
            user_intent: "Extend the memory tool with a new action",
            files_read: &["core/src/memory.rs".into(), "core/src/tools.rs".into()],
            files_changed: &["core/src/memory.rs".into(), "core/src/tools.rs".into()],
            symbols: &["MemoryEntry".into(), "memory_tool".into()],
            tools_used: &["read_file".into(), "edit".into(), "bash".into()],
            diagnostics: &[],
            tests_run: &["cargo test memory".into()],
        });
        let b = build_fingerprint(&FingerprintInputs {
            user_intent: "Add a memory tool action for deprecate",
            files_read: &["core/src/tools.rs".into(), "core/src/memory.rs".into()],
            files_changed: &["core/src/tools.rs".into()],
            symbols: &["memory_tool".into(), "MemoryEntry".into()],
            // Different tools chosen by the agent.
            tools_used: &["grep".into(), "patch".into(), "subagent".into()],
            diagnostics: &[],
            tests_run: &["cargo test memory".into()],
        });
        assert_eq!(a.intent, "extend-memory-tool");
        assert_eq!(b.intent, "extend-memory-tool");
        let sim = fingerprint_similarity(&a, &b);
        assert!(sim >= 0.55, "expected similar fingerprints, got {sim}");
    }

    #[test]
    fn unrelated_tasks_have_low_similarity() {
        let a = build_fingerprint(&FingerprintInputs {
            user_intent: "Fix TUI rendering glitch",
            files_read: &["tui/render.go".into()],
            files_changed: &["tui/render.go".into()],
            symbols: &["Render".into()],
            tools_used: &["edit".into()],
            diagnostics: &[],
            tests_run: &[],
        });
        let b = build_fingerprint(&FingerprintInputs {
            user_intent: "Add OpenAI provider",
            files_read: &["core/src/provider.rs".into()],
            files_changed: &["core/src/provider.rs".into()],
            symbols: &["ProviderConfig".into()],
            tools_used: &["edit".into()],
            diagnostics: &[],
            tests_run: &[],
        });
        let sim = fingerprint_similarity(&a, &b);
        assert!(
            sim < 0.4,
            "unrelated tasks should not match strongly: {sim}"
        );
    }
}
