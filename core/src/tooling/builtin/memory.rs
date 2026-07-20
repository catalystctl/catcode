use crate::config::Config;
use crate::tools::Outcome;
use serde_json::Value;

pub(crate) fn knowledge_tool(args: &Value, cfg: &Config) -> Outcome {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if action.is_empty() {
        return Outcome::err("knowledge requires action");
    }
    match crate::knowledge_tool::dispatch(action, args, &cfg.workspace) {
        Ok(s) => Outcome::ok(s),
        Err(e) => Outcome::err(e),
    }
}

pub(crate) fn memory_tool(args: &Value, cfg: &Config) -> Outcome {
    use crate::memory::{Importance, Scope};
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
    let scope = Scope::parse(
        args.get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("workspace"),
    );
    let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
    let importance = Importance::parse(
        args.get("importance")
            .and_then(|v| v.as_str())
            .unwrap_or("normal"),
    );
    match action {
        "save" => {
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let mem_type = args.get("type").and_then(|v| v.as_str()).unwrap_or("note");
            let mut description = args
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if name.trim().is_empty() {
                return Outcome::err("memory save requires 'name'");
            }
            if content.trim().is_empty() {
                return Outcome::err("memory save requires 'content'");
            }
            if description.trim().is_empty() {
                description = content
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or("")
                    .chars()
                    .take(100)
                    .collect();
            }
            let replaces = args
                .get("replaces")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            // Prefer accumulation: if the name already exists, append instead of
            // clobbering (auto-reflect often re-saves the same topic).
            let mut out = if crate::memory::memory_exists_scoped(&cfg.workspace, scope, name) {
                memory_append_inner(
                    &cfg.workspace,
                    scope,
                    name,
                    content,
                    mem_type,
                    &description,
                    importance,
                    force,
                    true,
                )
            } else {
                match crate::memory_hygiene::gate_write(
                    &cfg.workspace,
                    scope,
                    name,
                    content,
                    mem_type,
                    importance,
                    force,
                ) {
                    Ok(warnings) => match crate::memory::save_memory_scoped_with_importance(
                        &cfg.workspace,
                        scope,
                        name,
                        content,
                        mem_type,
                        &description,
                        importance,
                    ) {
                        Ok(p) => {
                            let id = p
                                .file_stem()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            let mut msg =
                                format!("saved {} memory '{name}' (id: {id})", scope.as_str());
                            for w in warnings {
                                msg.push_str("\nnote: ");
                                msg.push_str(&w);
                            }
                            Outcome::ok(memory_write_ok(&cfg.workspace, msg))
                        }
                        Err(e) => Outcome::err(e),
                    },
                    Err(e) => Outcome::err(e),
                }
            };
            // Deprecate the superseded memory (if any) AFTER a successful
            // save/append, so a failed write doesn't orphan a deprecation.
            if !replaces.is_empty() {
                match crate::memory::mark_memory_deprecated_any(
                    &cfg.workspace,
                    &replaces,
                    Some(name),
                ) {
                    Ok(()) => out.output.push_str(&format!(
                        "\nmarked '{}' deprecated (superseded by '{name}') — excluded from catalog/relevant surfaces",
                        replaces
                    )),
                    Err(e) => out.output.push_str(&format!(
                        "\nnote: could not mark '{}' deprecated: {e}",
                        replaces
                    )),
                }
            }
            out
        }
        "append" => {
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let mem_type = args.get("type").and_then(|v| v.as_str()).unwrap_or("note");
            let mut description = args
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if name.trim().is_empty() {
                return Outcome::err("memory append requires 'name'");
            }
            if content.trim().is_empty() {
                return Outcome::err("memory append requires 'content'");
            }
            if description.trim().is_empty() {
                description = content
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or("")
                    .chars()
                    .take(100)
                    .collect();
            }
            memory_append_inner(
                &cfg.workspace,
                scope,
                name,
                content,
                mem_type,
                &description,
                importance,
                force,
                false,
            )
        }
        "list" => {
            // Catalog view (name + one-line). Use get for full bodies.
            let scope_str = args.get("scope").and_then(|v| v.as_str()).unwrap_or("");
            let entries = if scope_str.is_empty() {
                crate::memory::scan_all_memories(&cfg.workspace)
            } else {
                crate::memory::scan_memories_scoped(&cfg.workspace, scope)
            };
            if entries.is_empty() {
                return Outcome::ok("(no memories)");
            }
            let mut out = String::from("Memory catalog (use action=get with id for full text):\n");
            for m in &entries {
                let id = m
                    .path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let blurb = if !m.description.is_empty() {
                    m.description.clone()
                } else {
                    m.content
                        .lines()
                        .map(str::trim)
                        .find(|l| !l.is_empty())
                        .unwrap_or("")
                        .to_string()
                };
                let desc = if blurb.is_empty() {
                    String::new()
                } else {
                    format!(": {blurb}")
                };
                let dep = if m.deprecated { " [DEPRECATED]" } else { "" };
                let stale = match m.status {
                    crate::memory::MemoryStatus::NeedsVerification | crate::memory::MemoryStatus::Stale => " [STALE]",
                    crate::memory::MemoryStatus::Candidate => " [CANDIDATE]",
                    crate::memory::MemoryStatus::Rejected => " [REJECTED]",
                    _ => "",
                };
                out.push_str(&format!(
                    "- {} [id: {}] ({}, {}, {}, status={}, conf={:.2}){}{}{}
",
                    m.name,
                    id,
                    m.mem_type,
                    m.scope.as_str(),
                    m.importance.as_str(),
                    m.status.as_str(),
                    m.confidence,
                    desc,
                    dep,
                    stale
                ));
            }
            Outcome::ok(out.trim_end().to_string())
        }
        "get" => {
            let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let id = if id.trim().is_empty() {
                args.get("name").and_then(|v| v.as_str()).unwrap_or("")
            } else {
                id
            };
            if id.trim().is_empty() {
                return Outcome::err("memory get requires 'id' (or 'name')");
            }
            let scope_str = args.get("scope").and_then(|v| v.as_str()).unwrap_or("");
            let result = if scope_str.is_empty() {
                crate::memory::get_memory(&cfg.workspace, id)
            } else {
                crate::memory::get_memory_scoped(&cfg.workspace, scope, id)
            };
            match result {
                Ok(m) => {
                    let id = m
                        .path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    crate::memory_recall::record_get(&cfg.workspace, &id);
                    let banner = if m.deprecated {
                        let sup = m
                            .superseded_by
                            .as_deref()
                            .map(|s| format!(" (superseded by '{s}')"))
                            .unwrap_or_default();
                        format!(
                            "⚠ DEPRECATED{sup} — this memory is superseded/excluded from recall. \
                             Prefer its successor; forget if obsolete.\n\n"
                        )
                    } else {
                        String::new()
                    };
                    let meta = format!(
                        "status={} conf={:.2} schema_v={} refs_files={} refs_symbols={}",
                        m.status.as_str(),
                        m.confidence,
                        m.schema_version,
                        m.ref_files.len(),
                        m.ref_symbols.len(),
                    );
                    let verified = m
                        .last_verified_at
                        .map(|ts| format!(" last_verified_at={ts}"))
                        .unwrap_or_default();
                    Outcome::ok(format!(
                        "{banner}# {} [id: {}] ({}, {}, {})
{meta}{verified}
{}

{}",
                        m.name,
                        id,
                        m.mem_type,
                        m.scope.as_str(),
                        m.importance.as_str(),
                        if m.description.is_empty() {
                            "(no description)".to_string()
                        } else {
                            m.description.clone()
                        },
                        m.content.trim_end()
                    ))
                }
                Err(e) => Outcome::err(e),
            }
        }
        "forget" => {
            let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if id.trim().is_empty() {
                return Outcome::err("memory forget requires 'id' (the memory slug/name)");
            }
            let scope_str = args.get("scope").and_then(|v| v.as_str()).unwrap_or("");
            let result = if scope_str.is_empty() {
                crate::memory::forget_memory_any(&cfg.workspace, id)
            } else {
                crate::memory::forget_memory_scoped(&cfg.workspace, scope, id)
            };
            match result {
                Ok(()) => Outcome::ok(format!("forgot memory '{id}'")),
                Err(e) => Outcome::err(e),
            }
        }
        "consolidate" => {
            let scope_opt = if args
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .is_empty()
            {
                None
            } else {
                Some(scope)
            };
            match crate::memory_hygiene::consolidate(&cfg.workspace, scope_opt) {
                Ok(report) => Outcome::ok(memory_write_ok(&cfg.workspace, report.message)),
                Err(e) => Outcome::err(e),
            }
        }
        "stats" => {
            let s = crate::memory_recall::summary_json(&cfg.workspace);
            let hit = s
                .get("relevant_hit_rate")
                .and_then(|v| v.as_f64())
                .map(|f| format!("{:.0}%", f * 100.0))
                .unwrap_or_else(|| "n/a".into());
            let syn = s
                .get("synonym_recovery_rate")
                .and_then(|v| v.as_f64())
                .map(|f| format!("{:.0}%", f * 100.0))
                .unwrap_or_else(|| "n/a".into());
            Outcome::ok(format!(
                "Memory recall stats (workspace):\n\
                 - turns tracked: {}\n\
                 - relevant offers/gets/misses: {}/{}/{}\n\
                 - relevant hit rate: {hit}\n\
                 - synonym-miss offers/recovered: {}/{}\n\
                 - synonym recovery rate: {syn}\n\
                 (synonym misses = body matched the prompt but name/description did not — \
                 Milestone 4 embedding trigger)",
                s.get("turns").and_then(|v| v.as_u64()).unwrap_or(0),
                s.get("relevant_offers")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                s.get("relevant_gets").and_then(|v| v.as_u64()).unwrap_or(0),
                s.get("relevant_misses")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                s.get("synonym_miss_offers")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                s.get("synonym_miss_gets")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
            ))
        }
        "deprecate" => {
            let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let id = if id.trim().is_empty() {
                args.get("name").and_then(|v| v.as_str()).unwrap_or("")
            } else {
                id
            };
            if id.trim().is_empty() {
                return Outcome::err("memory deprecate requires 'id' (or 'name')");
            }
            let sup_raw = args
                .get("superseded_by")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let sup = if sup_raw.trim().is_empty() {
                None
            } else {
                Some(sup_raw)
            };
            let scope_str = args.get("scope").and_then(|v| v.as_str()).unwrap_or("");
            let result = if scope_str.is_empty() {
                crate::memory::mark_memory_deprecated_any(&cfg.workspace, id, sup)
            } else {
                crate::memory::mark_memory_deprecated(&cfg.workspace, scope, id, sup)
            };
            match result {
                Ok(()) => Outcome::ok(format!(
                    "marked memory '{id}' deprecated (excluded from catalog/relevant surfaces)"
                )),
                Err(e) => Outcome::err(e),
            }
        }
        "migrate" => {
            // One-time rewrite of stale project-name refs (umans-harness →
            // catalyst-code, UMANS_CORE → CATALYST_CODE) across both scopes.
            // Idempotent; preserves all metadata.
            match crate::memory::migrate_memories(&cfg.workspace) {
                Ok(report) => Outcome::ok(memory_write_ok(&cfg.workspace, report.message)),
                Err(e) => Outcome::err(e),
            }
        }
        other => Outcome::err(format!(
            "memory: unknown action '{other}' (save|append|list|get|forget|consolidate|stats|deprecate|migrate)"
        )),
    }
}

fn memory_append_inner(
    workspace: &std::path::Path,
    scope: crate::memory::Scope,
    name: &str,
    content: &str,
    mem_type: &str,
    description: &str,
    importance: crate::memory::Importance,
    force: bool,
    redirected_from_save: bool,
) -> Outcome {
    match crate::memory_hygiene::gate_write(
        workspace, scope, name, content, mem_type, importance, force,
    ) {
        Err(e) => Outcome::err(e),
        Ok(warnings) => {
            match crate::memory::append_memory_scoped(
                workspace,
                scope,
                name,
                content,
                mem_type,
                description,
                8192,
            ) {
                Ok(p) => {
                    let id = p
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let mut base = if redirected_from_save {
                        format!(
                            "name exists — appended to {} memory '{name}' (id: {id}) instead of overwriting",
                            scope.as_str()
                        )
                    } else {
                        format!("appended to {} memory '{name}' (id: {id})", scope.as_str())
                    };
                    for w in warnings {
                        base.push_str("\nnote: ");
                        base.push_str(&w);
                    }
                    Outcome::ok(memory_write_ok(workspace, base))
                }
                Err(e) => Outcome::err(e),
            }
        }
    }
}

fn memory_write_ok(workspace: &std::path::Path, msg: String) -> String {
    let n = crate::memory::memory_count(workspace);
    let mut out = if n >= crate::memory::SAVE_COUNT_WARN_THRESHOLD {
        format!(
            "{msg}\nnote: {n} memories stored — prefer append/merge/forget; standing catalog is capped"
        )
    } else {
        msg
    };
    if let Some(cons) = crate::memory_hygiene::maybe_auto_consolidate(workspace) {
        out.push('\n');
        out.push_str(&cons);
    }
    out
}
