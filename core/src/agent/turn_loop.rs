use crate::*;

pub(crate) async fn run_turn(
    st: &Arc<State>,
    client: &reqwest::Client,
    run: RunContext,
    model: String,
    prompt: String,
    effort: String,
    images: Option<Vec<String>>,
    cancel: CancellationToken,
) {
    // Remember the model the user selected so the manual `/compact` command
    // can size its reclaim budget against the right context window.
    *st.last_model.lock().await = Some(model.clone());
    // Lifecycle hook: notify plugins a session/turn is starting. Best-effort
    // and never blocks the turn.
    dispatch_lifecycle(st, "session_start").await;
    st.auto_checkpoint_taken
        .store(false, std::sync::atomic::Ordering::Relaxed);

    // Speculative readonly prefetch: warm tool_cache from recurring file
    // categories (never mutates the workspace).
    speculative_prefetch(st, &prompt).await;

    // If the conversation was left mid-`ask` by a prior core restart (the
    // assistant `ask` tool_call is persisted but no tool result exists),
    // re-present the question before the turn proceeds. Without this the
    // session is wedged (no modal) and the orphan-sanitizer would later
    // insert a synthetic EMPTY result, losing the question. No-op on the
    // common case of no trailing unanswered ask. Idempotent: once the ask
    // has a result, `find_trailing_unanswered_ask` returns None.
    resume_pending_ask(st, &cancel).await;

    // Clear the last-turn metrics at turn entry so a panic before finalization
    // can't leak the PRIOR turn's numbers to this turn's `session_stop` hook
    // (which fires unconditionally from the panic guard). A completed turn sets
    // it fresh at finalization; a failed turn leaves it None and the telemetry
    // plugin skips rather than recording a phantom turn.
    *st.last_turn_metrics.lock().await = None;

    // Vision handoff (pre_turn) and other plugins may remap the model for
    // this turn; keep a mutable binding so a swap propagates to the request loop.
    let mut model = model;

    // Auto-reflect turn-local state (SELF_LEARNING §11 deterministic seam). The
    // shape (tool names + file categories) is accumulated as tools run; at the
    // first `finish`/natural completion of a non-trivial turn it is logged to
    // the recurrence store and a reflection continuation is injected so durable
    // facts/patterns get persisted without relying on the model remembering to.
    // `reflected` prevents re-entry: the reflect's own `finish` exits for real.
    let mut reflected = false;
    let mut turn_tool_calls: u32 = 0;
    let mut shape_tools: Vec<String> = Vec::new();
    let mut shape_files: Vec<String> = Vec::new();

    // Ensure system prompt is present; persist every finalized message to the session file.
    let mut init_est_add = 0u64;
    // Expand `@<path>` file mentions so the model sees the referenced file's
    // contents directly (no `read_file` round-trip) — mirroring how
    // `apply_skill` inlines a skill body. The transcript keeps the concise
    // `@path` the user typed (the TUI/web already logged the raw text).
    let (user_text, attached_files) = {
        let c = st.cfg.read().await;
        expand_file_mentions(&prompt, &c.workspace, c.max_read_bytes)
    };
    if !attached_files.is_empty() {
        emit(&Event::new("info").with(
            "message",
            json!(format!(
                "attached {} file(s) from @mentions: {}",
                attached_files.len(),
                attached_files.join(", ")
            )),
        ));
    }
    // P0-H1: pre_input — plugins may rewrite/deny user text before it becomes
    // a conversation Message. Deny aborts the turn with an error event.
    let user_text = match run_pre_input(st, &user_text).await {
        Ok(t) => t,
        Err(reason) => {
            emit(&Event::new("error").with("message", json!(reason)));
            emit(&Event::new("done"));
            return;
        }
    };
    // P0-H1: turn_start — advisory turn-boundary signal (distinct from vision
    // pre_turn and from session_start).
    dispatch_lifecycle(st, "turn_start").await;
    {
        let mut conv = st.conversation.lock().await;
        if conv.is_empty() {
            let (workspace, auto_reflect) = {
                let c = st.cfg.read().await;
                (c.workspace.clone(), c.auto_reflect)
            };
            let sys_msg = Message::system(build_main_system_prompt(
                &workspace,
                &st.plugin_manager,
                auto_reflect,
            ));
            init_est_add += estimate_message_tokens(&sys_msg);
            conv.push(sys_msg);
            if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                session::append(p, &conv[0]);
            }
        }
        // Build the user message. If images are attached and vision is allowed,
        // emit a multimodal content array (text + image_url parts).
        let allow_vision = st.cfg.read().await.allow_vision;
        let user_msg = match (&images, allow_vision) {
            (Some(imgs), true) if !imgs.is_empty() => {
                let mut parts: Vec<ContentPart> = vec![ContentPart::Text {
                    text: user_text.clone(),
                }];
                for img in imgs {
                    let url = image_to_data_url(img);
                    parts.push(ContentPart::Image {
                        image_url: ImageUrl { url, detail: None },
                    });
                }
                Message::user_multimodal(parts)
            }
            _ => Message::user(user_text.clone()),
        };
        init_est_add += estimate_message_tokens(&user_msg);
        conv.push(user_msg);
        if let Some(p) = st.cfg.read().await.session_file.as_ref() {
            session::append(p, conv.last().unwrap());
        }
    }
    if init_est_add > 0 {
        *st.estimated_tokens.lock().await += init_est_add;
    }

    // Seed the rolling work-state's goal from the user's first substantive
    // prompt. No-op once a goal is set; slash commands / tiny prompts ignored.
    maybe_seed_work_state_goal(st, &prompt).await;

    // Vision handoff (pre_turn hook): let plugins inspect the upcoming turn
    // (model + attached images) and optionally remap the model before the first
    // request. Advisory — a broken/missing hook or `allow:false` never blocks
    // the turn; only `modify.model` (validated against discovered models) is honored.
    {
        let has_images = images.as_ref().is_some_and(|v| !v.is_empty());
        let image_count = images.as_ref().map_or(0, |v| v.len());
        let vc = st.vision.read().await.clone();
        let models_snapshot = st.models.read().await.clone();
        let active_provider = models_snapshot
            .iter()
            .find(|m| m.id == model.as_str())
            .map(|m| m.provider.clone())
            .unwrap_or_default();
        let recommended = vision::recommend_vision_model(&model, &models_snapshot, &vc);
        let models_json: Vec<Value> = models_snapshot
            .iter()
            .map(|m| {
                json!({
                    "id": m.id.clone(),
                    "vision": m.vision || vc.has_vision(m.id.as_str()),
                    "provider": m.provider.clone(),
                    "cost_rank": vision::vision_cost_rank(&m.id),
                })
            })
            .collect();
        let (workspace, session_id) = {
            let cfg = st.cfg.read().await;
            (
                cfg.workspace.display().to_string(),
                cfg.session_file
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            )
        };
        let original_model = model.clone();

        // First-class enable gate (default ON). When off, skip plugin remap and
        // tell the user why images won't be seen.
        if has_images && !vc.enabled {
            let current_has_vision = models_snapshot
                .iter()
                .find(|m| m.id == model.as_str())
                .map(|m| m.vision || vc.has_vision(m.id.as_str()))
                .unwrap_or(false);
            if !current_has_vision {
                emit(&Event::new("info").with(
                    "message",
                    json!(format!(
                        "image attached but vision handoff is disabled and '{}' lacks vision; enable it in Settings / /vision (recommended), or pick a vision model",
                        model
                    )),
                ));
            }
        } else if has_images {
            for (plugin_name, config) in &st.plugin_manager.get_hook_configs("pre_turn") {
                let turn_args = json!({
                    "model": model.clone(),
                    "has_images": has_images,
                    "image_count": image_count,
                    "models": models_json,
                    "vision_model": vc.vision_model.clone(),
                    "enabled": vc.enabled,
                    "provider": active_provider,
                    "recommended_vision_model": recommended,
                });
                let ctx = plugins::build_context(
                    "pre_turn",
                    "",
                    &workspace,
                    Some(&turn_args),
                    &session_id,
                    config.pass_args,
                );
                let result =
                    execute_plugin_hook_logged(st, "pre_turn", plugin_name, config, &ctx).await;
                if let Some(new_model) = result
                    .modify
                    .as_ref()
                    .and_then(|m| m.get("model"))
                    .and_then(|v| v.as_str())
                {
                    if new_model != model.as_str() {
                        let valid = models_snapshot.iter().any(|m| m.id.as_str() == new_model);
                        if valid {
                            let why = if result.reason.is_empty() {
                                "vision handoff".to_string()
                            } else {
                                result.reason.clone()
                            };
                            emit(&Event::new("info").with(
                                "message",
                                json!(format!(
                                    "vision handoff: {} → {} ({})",
                                    model, new_model, why
                                )),
                            ));
                            st.logger.log(
                                "vision_handoff",
                                json!({
                                    "from": model, "to": new_model, "plugin": plugin_name.clone(), "reason": why
                                }),
                            );
                            model = new_model.to_string();
                        } else {
                            emit(&Event::new("info").with(
                                "message",
                                json!(format!(
                                    "vision handoff ignored: '{}' is not a discovered model",
                                    new_model
                                )),
                            ));
                        }
                    }
                }
            }
            // No vision plugin handed off an image-bearing turn on a non-vision
            // model. Prefer the Rust-ranked recommendation (works even if the
            // plugin is missing / python3 absent), else warn.
            if model == original_model {
                let current_has_vision = models_snapshot
                    .iter()
                    .find(|m| m.id == model.as_str())
                    .map(|m| m.vision || vc.has_vision(m.id.as_str()))
                    .unwrap_or(false);
                if !current_has_vision {
                    if let Some(rec) = recommended.as_ref() {
                        if models_snapshot
                            .iter()
                            .any(|m| m.id.as_str() == rec.as_str())
                        {
                            emit(&Event::new("info").with(
                                "message",
                                json!(format!(
                                    "vision handoff: {} → {} (cheapest same-provider)",
                                    model, rec
                                )),
                            ));
                            st.logger.log(
                                "vision_handoff",
                                json!({
                                    "from": model, "to": rec, "plugin": "core",
                                    "reason": "cheapest same-provider"
                                }),
                            );
                            model = rec.clone();
                        }
                    } else {
                        emit(&Event::new("info").with("message", json!(format!(
                            "image attached but '{}' lacks vision and no same-provider vision model is available to hand off to; use /vision to set one (or select a vision model with /model)",
                            model
                        ))));
                    }
                }
            }
        }
    }

    // Main agent tool list: core built-ins (always) + session-enabled deferred
    // tools (via load_tools) + goal_write_plan when planning, MERGED with tools
    // declared by enabled plugins, then filtered by every plugin's
    // `disable_tools`. Subagent-only tools (contact_supervisor/intercom) stay
    // hidden on the main agent. Three plugin capabilities converge here:
    //   • ADD       — a plugin `tools` entry adds a new capability.
    //   • OVERRIDE  — a plugin tool with `override:true` whose name matches a
    //                  built-in REPLACES that built-in: the plugin's declared
    //                  schema is shown to the model and calls route to the
    //                  plugin handler (see the dispatch below).
    //   • REMOVE    — `disable_tools` names are dropped from the final list
    //                  (built-in OR override). `disable_tools` is the strongest
    //                  lever: a disabled name is gone, period.
    // A plugin tool that merely collides with a built-in name (no `override`)
    // is still skipped — the built-in wins, unchanged.
    let overridden = st.plugin_manager.overridden_tool_names();
    let disabled = st.plugin_manager.disabled_tools();
    let enabled_deferred = st.enabled_deferred_tools.lock().await.clone();
    let goal_planning = {
        let g = st.goal.lock().await;
        g.phase == goal::GoalPhase::Planning
    };
    let mut reserved: std::collections::HashSet<String> = std::collections::HashSet::new();
    reserved.insert("contact_supervisor".into());
    reserved.insert("intercom".into());
    let mut tool_defs: Vec<Value> = tools::definitions()
        .into_iter()
        .filter(|d| {
            let n = d
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            reserved.insert(n.to_string());
            // Hide the reserved subagent-only tools, AND any built-in a plugin
            // is overriding (its plugin version is added below instead).
            if n == "contact_supervisor" || n == "intercom" || overridden.contains(n) {
                return false;
            }
            // Core tools always; deferred only when session-enabled (or goal
            // planning needs goal_write_plan).
            if tools::is_core_tool(n) {
                return true;
            }
            if n == "goal_write_plan" && goal_planning {
                return true;
            }
            enabled_deferred.contains(n)
        })
        .collect();
    for d in st.plugin_manager.tool_definitions() {
        let n = d
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // An override tool replaces the built-in (already excluded above), so
        // it's always added and claims the name. A plain custom tool is added
        // only if its name isn't already taken (built-in or another plugin).
        let is_override = overridden.contains(n);
        if !is_override && !reserved.insert(n.to_string()) {
            eprintln!(
                "[plugins] tool '{}' collides with a built-in or already-registered tool; skipping",
                n
            );
            continue;
        }
        tool_defs.push(d);
    }
    // REMOVE: `disable_tools` is a final, composition-winning filter — a
    // disabled name vanishes whether it was a built-in, an override, or a
    // custom plugin tool. (No-op when no plugin disables anything.)
    if !disabled.is_empty() {
        tool_defs.retain(|d| {
            d.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .is_none_or(|n| !disabled.contains(n))
        });
    }
    let mut timer = TurnTimer::new();

    // Working conversation buffer for this turn: cloned once here, then kept
    // across agentic loop iterations. Tool/assistant appends update both this
    // buffer and `st.conversation`; we only re-sync from state after parallel
    // waves / reflect that mutate state without going through `messages`.
    let mut messages = st.conversation.lock().await.clone();

    // Idle compaction: if 60+ minutes since the last turn completed, compact the
    // conversation so the next turn starts lean. Uses the same summarize strategy
    // as the threshold path; falls back to naive drop-oldest without an api key.
    {
        let last = *st.last_turn_time.lock().await;
        let cfg = st.cfg.read().await.clone();
        if cfg.auto_compact && last.elapsed().as_secs() > 3600 {
            let est = grounded_estimate(
                &messages,
                *st.last_real_prompt_tokens.lock().await,
                *st.conv_len_at_last_real.lock().await,
            );
            let (idle_ctx, idle_max_tokens) = st
                .models
                .read()
                .await
                .iter()
                .find(|m| m.id == model)
                .map(|m| (m.context_window as u64, m.max_tokens))
                .unwrap_or((200_000, 8_192));
            let policy = context_policy(
                &messages,
                idle_ctx,
                idle_max_tokens,
                cfg.context_compact_at,
                cfg.context_digest_at,
            );
            // Idleness alone is not token pressure. Only compact when this same
            // conversation would cross the normal model-aware threshold.
            if should_auto_compact(cfg.auto_compact, est, messages.len(), policy) {
                emit(
                    &Event::new("compacting")
                        .with("before_tokens", json!(est))
                        .with("trigger", json!("idle_threshold"))
                        .with("context_window", json!(idle_ctx))
                        .with("threshold_tokens", json!(policy.compact_threshold))
                        .with("hard_limit_tokens", json!(policy.hard_limit))
                        .with("response_reserve_tokens", json!(policy.response_reserve))
                        .with("utilization_pct", json!(utilization_pct(est, idle_ctx))),
                );
                dispatch_lifecycle(st, "pre_compact").await;
                let rp = st.resolve_provider_for_model(&model).await;
                let summary_chars = if rp.api_key.is_some() {
                    let mp = st.plugin_manager.memory_provider();
                    compact_with_summary(
                        client,
                        &cfg,
                        &rp,
                        &model,
                        &mut messages,
                        &cancel,
                        false,
                        idle_ctx,
                        cfg.compact_instructions.as_deref(),
                        mp.as_ref(),
                    )
                    .await
                } else {
                    compact_conversation(&mut messages, idle_ctx);
                    0
                };
                *st.conversation.lock().await = messages.clone();
                if let Some(p) = cfg.session_file.as_ref() {
                    session::rewrite(p, &messages);
                }
                let after_est = estimate_messages_tokens(&messages);
                *st.estimated_tokens.lock().await = after_est;
                // Idle compaction rewrote history; the old real baseline is stale.
                st.invalidate_real_token_baseline().await;
                emit(
                    &Event::new("compacted")
                        .with("before_tokens", json!(est))
                        .with("after_tokens", json!(after_est))
                        .with("summary_chars", json!(summary_chars))
                        .with("context_window", json!(idle_ctx))
                        .with("threshold_tokens", json!(policy.compact_threshold))
                        .with("hard_limit_tokens", json!(policy.hard_limit))
                        .with("within_limit", json!(after_est <= policy.hard_limit)),
                );
                st.compaction_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if after_est > policy.hard_limit {
                    emit(&Event::new("error").with(
                        "message",
                        json!(format!(
                            "context remains too large after compaction ({after_est} > safe limit {}); remove or split an oversized recent message",
                            policy.hard_limit
                        )),
                    ));
                    emit(&Event::new("done"));
                    return;
                }
            }
        }
    }

    loop {
        if cancel.is_cancelled() {
            sync_session_file(st).await;
            emit_aborted_done();
            return;
        }
        // Session token budget (hard ceiling across the whole session, not per turn).
        // 0 = unlimited. Trips before the request so we don't blow past a cost cap.
        let budget = st.cfg.read().await.max_session_tokens;
        if budget > 0 {
            let spent = *st.tokens_in.lock().await + *st.tokens_out.lock().await;
            if spent >= budget {
                emit(&Event::new("error").with(
                    "message",
                    json!(format!(
                        "session token budget exhausted ({spent} >= {budget}); start a new session"
                    )),
                ));
                emit(&Event::new("done"));
                return;
            }
        }

        // Resolve the provider for this turn. In the multi-login model the
        // turn routes to the selected model's owning provider (so `/models`
        // can mix providers); falls back to the active/legacy provider for
        // models without a provider tag. Errors out if no API key is available
        // for the resolved provider (runtime key -> config literal -> env var).
        let provider = {
            let rp = st.resolve_provider_for_model(&model).await;
            match rp.api_key.as_ref() {
                Some(_) => rp,
                None => {
                    emit(&Event::new("error").with(
                        "message",
                        json!(format!(
                            "no API key set for provider '{}'; use /login to log in",
                            rp.name
                        )),
                    ));
                    sync_session_file(st).await;
                    emit(&Event::new("done"));
                    return;
                }
            }
        };

        let cfg = st.cfg.read().await.clone();
        // Context window management: compact at the configured threshold
        // (default 90%) or sooner when model-aware response headroom requires
        // it. `auto_compact` gates every automatic history rewrite: pressure
        // digest, threshold compaction, idle compaction, and subagent reclaim.
        // When false, the user must /compact manually (or /clear).
        // `messages` is the turn-local working buffer (cloned once before the loop).
        let (model_ctx, thinking_levels, max_tokens) = st
            .models
            .read()
            .await
            .iter()
            .find(|m| m.id == model)
            .map(|m| {
                (
                    m.context_window as u64,
                    m.thinking_levels.clone(),
                    m.max_tokens,
                )
            })
            .unwrap_or((200_000, Vec::new(), 8_192));
        // Anchor on the endpoint's REAL `prompt_tokens` from the last request
        // (the authoritative count of the conversation as the model tokenized it —
        // system + messages + tool-call framing the char/4 heuristic cannot see)
        // and only char/4-estimate the messages appended since. This is far more
        // accurate than re-estimating the whole history every loop iteration, so
        // compaction fires at the right time instead of drifting late into a
        // context-window 400. Falls back to a full char/4 estimate when no real
        // usage has been seen yet (first turn) or right after compaction.
        let last_real = *st.last_real_prompt_tokens.lock().await;
        let len_at = *st.conv_len_at_last_real.lock().await;
        let mut est = grounded_estimate(&messages, last_real, len_at);
        *st.estimated_tokens.lock().await = est;
        let policy = context_policy(
            &messages,
            model_ctx,
            max_tokens,
            cfg.context_compact_at,
            cfg.context_digest_at,
        );
        // Soft digest: collapse stale, large tool results AND oversized tool-call
        // arguments into one-line digests well before the compaction threshold so
        // they stop being re-sent verbatim on every turn. Keep-window is sized by
        // token budget (20% of the context window), not a fixed message count —
        // a few huge recent results no longer block reclaim of everything older.
        // Idempotent; tool_call_id + role preserved so pairing stays intact.
        if should_auto_digest(cfg.auto_compact, est, policy) {
            let before_est = est;
            let changed = {
                let mut cache = st.tool_output_cache.lock().await;
                soft_digest_conversation(&mut messages, model_ctx, Some(&mut cache))
            };
            if changed > 0 {
                *st.conversation.lock().await = messages.clone();
                if let Some(p) = cfg.session_file.as_ref() {
                    session::rewrite(p, &messages);
                }
                est = estimate_messages_tokens(&messages);
                *st.estimated_tokens.lock().await = est;
                // Digest rewrote message contents, so the real prompt_tokens
                // baseline no longer matches — drop it until the next request.
                st.invalidate_real_token_baseline().await;
                st.logger.log(
                    "digested",
                    json!({ "results": changed, "before_tokens": before_est, "after_tokens": est }),
                );
                emit(
                    &Event::new("digested")
                        .with("results", json!(changed))
                        .with("before_tokens", json!(before_est))
                        .with("after_tokens", json!(est))
                        .with("trigger", json!("pressure"))
                        .with("context_window", json!(model_ctx))
                        .with("threshold_tokens", json!(policy.digest_threshold))
                        .with("hard_limit_tokens", json!(policy.hard_limit))
                        .with(
                            "utilization_pct",
                            json!(utilization_pct(before_est, model_ctx)),
                        ),
                );
            }
        }
        let force_summarize = est > policy.hard_limit;
        if should_auto_compact(cfg.auto_compact, est, messages.len(), policy) {
            emit(
                &Event::new("compacting")
                    .with("before_tokens", json!(est))
                    .with("trigger", json!("threshold"))
                    .with("context_window", json!(model_ctx))
                    .with("threshold_tokens", json!(policy.compact_threshold))
                    .with("hard_limit_tokens", json!(policy.hard_limit))
                    .with("response_reserve_tokens", json!(policy.response_reserve))
                    .with("safety_margin_tokens", json!(policy.safety_margin))
                    .with("utilization_pct", json!(utilization_pct(est, model_ctx))),
            );
            dispatch_lifecycle(st, "pre_compact").await;
            let summary_chars = {
                let mp = st.plugin_manager.memory_provider();
                compact_with_summary(
                    client,
                    &cfg,
                    &provider,
                    &model,
                    &mut messages,
                    &cancel,
                    force_summarize,
                    model_ctx,
                    cfg.compact_instructions.as_deref(),
                    mp.as_ref(),
                )
                .await
            };
            *st.conversation.lock().await = messages.clone();
            if let Some(p) = cfg.session_file.as_ref() {
                session::rewrite(p, &messages);
            }
            let after_est = estimate_messages_tokens(&messages);
            *st.estimated_tokens.lock().await = after_est;
            // Compaction rewrote history; the old real baseline is stale.
            st.invalidate_real_token_baseline().await;
            emit(
                &Event::new("compacted")
                    .with("before_tokens", json!(est))
                    .with("after_tokens", json!(after_est))
                    .with("summary_chars", json!(summary_chars))
                    .with("context_window", json!(model_ctx))
                    .with("threshold_tokens", json!(policy.compact_threshold))
                    .with("hard_limit_tokens", json!(policy.hard_limit))
                    .with("within_limit", json!(after_est <= policy.hard_limit)),
            );
            st.compaction_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if after_est > policy.hard_limit {
                emit(&Event::new("error").with(
                    "message",
                    json!(format!(
                        "context remains too large after compaction ({after_est} > safe limit {}); remove or split an oversized recent message",
                        policy.hard_limit
                    )),
                ));
                sync_session_file(st).await;
                emit(&Event::new("done"));
                return;
            }
        }

        // Sanitize orphaned tool calls + malformed tool-call arguments right
        // before the request. Orphans can arise not only from context compaction
        // but from ANY turn that ended with an assistant `tool_calls` message
        // whose matching results weren't all appended — notably an aborted
        // approval, which `return`s after the assistant message (carrying ALL
        // its tool_calls) was already persisted but before results for the
        // aborted + remaining calls were appended. The next request would then
        // ship an orphaned `tool_calls` and the API rejects it with HTTP 400
        // "No tool output found for function call …", which bricks the session
        // (it repeats every turn). The scan is O(n) with tiny constants and a
        // strict no-op on clean turns; we persist back only when it actually
        // changed something, so clean turns pay just the scan (no clone, no
        // session rewrite). The subagent path already does this unconditionally
        // (subagent.rs) — this makes the main loop consistent with it.
        let orphan_fixes = provider::sanitize_orphaned_tool_calls(&mut messages);
        let fixed_args = provider::sanitize_tool_call_arguments(&mut messages);
        if orphan_fixes > 0 || fixed_args > 0 {
            *st.conversation.lock().await = messages.clone();
            if let Some(p) = cfg.session_file.as_ref() {
                session::rewrite(p, &messages);
            }
            if orphan_fixes > 0 {
                emit(&Event::new("info").with("message", json!(format!(
                    "inserted {orphan_fixes} synthetic tool result(s) for tool call(s) whose result was missing (e.g. after an aborted turn) — the conversation is valid again for the API"
                ))));
            }
            if fixed_args > 0 {
                emit(&Event::new("info").with("message", json!(format!(
                    "sanitized {fixed_args} malformed tool-call argument(s) to keep the conversation valid for the API"
                ))));
            }
        }
        // Best pre-stream estimate of this request's prompt size, grounded on the
        // endpoint's last real `prompt_tokens` when available. Passed to
        // stream_turn so the live footer percentage tracks reality while output
        // streams in (the real `usage` chunk at stream end then overwrites it).
        let prompt_est = grounded_estimate(
            &messages,
            *st.last_real_prompt_tokens.lock().await,
            *st.conv_len_at_last_real.lock().await,
        );
        // Transient tails (never persisted): relevant memories for this turn,
        // then rolling work-state. Both sit AFTER the stable conversation prefix
        // so updating them does not bust the provider prefix cache.
        let mut transient_tails = 0usize;
        let last_user = messages
            .iter()
            .rev()
            .find_map(|m| match m {
                Message::User { .. } => {
                    if let Some(t) = m.content_text() {
                        return Some(t.to_string());
                    }
                    // Multimodal user message: join text parts only.
                    m.content_parts().map(|parts| {
                        parts
                            .iter()
                            .filter_map(|p| match p {
                                message::ContentPart::Text { text } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                }
                _ => None,
            })
            .unwrap_or_default();
        // Skip relevance when a memory_provider plugin owns injection — it
        // already decided what belongs in the standing prompt.
        let mem_tail = {
            let has_provider = st.plugin_manager.has_memory_provider();
            if has_provider || last_user.trim().is_empty() {
                if has_provider {
                    st.logger.log(
                        "memory_retrieval",
                        json!({
                            "provider": "plugin",
                            "candidates": null,
                            "injected": null,
                            "duration_ms": 0,
                            "status": "delegated",
                        }),
                    );
                }
                String::new()
            } else {
                let ws = st.cfg.read().await.workspace.clone();
                let started = std::time::Instant::now();
                let candidates = memory::scan_all_memories(&ws)
                    .into_iter()
                    .filter(|memory| !memory.deprecated)
                    .count();
                let tail = relevant_memories_tail(&ws, &last_user);
                let injected = tail
                    .lines()
                    .filter(|line| line.trim_start().starts_with("- **"))
                    .count();
                st.logger.log(
                    "memory_retrieval",
                    json!({
                        "provider": "builtin",
                        "candidates": candidates,
                        "injected": injected,
                        "duration_ms": started.elapsed().as_millis() as u64,
                        "status": "completed",
                    }),
                );
                tail
            }
        };
        // Skill auto-suggestion: append a [RELEVANT SKILL] hint when a skill's
        // name+description semantically matches the prompt, so the agent can
        // apply it without remembering /skill:<name>. Sits with the
        // relevant-memories tail as one transient (non-prefix-cached) message.
        let mut tail = mem_tail;
        if !last_user.trim().is_empty() {
            let ws = st.cfg.read().await.workspace.clone();
            let pack = context_pack::build_context_pack(&ws, &last_user);
            if !pack.is_empty() {
                if tail.is_empty() {
                    tail = pack;
                } else {
                    tail.push_str("\n\n");
                    tail.push_str(&pack);
                }
            }
            if let Some(h) = subagent::relevant_skill_hint(&ws, &last_user) {
                // Utility signal: skill was retrieved (not yet proven followed).
                if let Some(start) = h.find(char::from_u32(39).unwrap()) {
                    if let Some(end) = h[start + 1..].find(char::from_u32(39).unwrap()) {
                        let name = &h[start + 1..start + 1 + end];
                        if !name.is_empty() {
                            let _ = skill_metrics::record_outcome(
                                name,
                                skill_metrics::OutcomeKind::Success,
                            );
                        }
                    }
                }
                if tail.is_empty() {
                    tail = h;
                } else {
                    tail.push_str("\n\n");
                    tail.push_str(&h);
                }
            }
        }
        if !tail.is_empty() {
            messages.push(Message::system(tail));
            transient_tails += 1;
        }
        let ws_msg = work_state_message(st).await;
        if let Some(msg) = &ws_msg {
            messages.push(msg.clone());
            transient_tails += 1;
        }
        // P0-H1: pre_agent_start — dynamic system-prompt surgery as a transient
        // system message (not persisted). Fail-open on hook errors.
        if let Some(dyn_prompt) = run_pre_agent_start(st).await {
            messages.push(Message::system(dyn_prompt));
            transient_tails += 1;
        }
        // P0-H1: pre_context — rewrite the message list before the LLM call.
        // Fail-open: invalid/oversized modify keeps prior messages.
        run_pre_context(st, &mut messages).await;
        // messages is already Vec<Message> — pass directly.
        let provider_result = provider::stream_turn(
            client,
            &provider,
            cfg.idle_timeout_secs,
            &model,
            &messages,
            &tool_defs,
            &effort,
            &thinking_levels,
            max_tokens,
            &cancel,
            &mut timer,
            prompt_est,
            false,
        )
        .await;
        if provider_result.is_err() {
            timer.finish_failed_provider_call();
        }
        if let Some(call_metrics) = timer.take_provider_call_metrics() {
            st.logger.log(
                "provider_request",
                json!({
                    "provider": &provider.name,
                    "provider_kind": provider.kind.as_str(),
                    "model": &model,
                    "duration_ms": call_metrics.duration_ms,
                    "ttft_ms": call_metrics.ttft_ms,
                    "stream_ms": call_metrics.stream_ms,
                    "status": if provider_result.is_ok() { "completed" } else { "failed" },
                }),
            );
        }
        let (assistant, _finish, tokens_in, tokens_out, cached_tokens) = match provider_result {
            Ok(v) => v,
            Err(e) => {
                st.logger.log("turn_error", json!({ "error": e }));
                sync_session_file(st).await;
                if e == "aborted" {
                    emit(&Event::new("aborted"));
                } else {
                    emit(&Event::new("error").with("message", json!(e)));
                }
                emit(&Event::new("done"));
                return;
            }
        };

        // Strip transient tails before recording the token baseline so
        // conv_len_at_last_real reflects the persisted conversation length
        // (without the transient messages) and grounded_estimate's delta slice
        // stays correct. On the error path above we `return` first, so the
        // transient messages are simply dropped along with `messages`.
        for _ in 0..transient_tails {
            messages.pop();
        }

        // Convert the assistant from OpenAI-shaped Value to Message.
        let assistant_msg = Message::try_from(&assistant).unwrap_or_else(|e| {
            emit(&Event::new("error").with("message", json!(format!("assistant parse: {e}"))));
            Message::assistant("")
        });

        // Anchor all future estimates on the endpoint's REAL `prompt_tokens` —
        // the exact count of `messages` as the model tokenized it (system +
        // history + tool-call framing). `messages` is exactly what we sent, so its
        // length marks where the real baseline stops and the char/4 delta begins;
        // the compaction trigger and live footer then reflect reality instead of
        // a whole-history char/4 guess. Only when the endpoint actually reports
        // usage (some don't); otherwise we keep the previous baseline.
        if tokens_in > 0 {
            *st.last_real_prompt_tokens.lock().await = Some(tokens_in);
            *st.conv_len_at_last_real.lock().await = messages.len();
        }

        // Accumulate token counts for /stats. When the endpoint omits usage
        // (tokens come back zero) estimate from the exchanged messages so the
        // session totals aren't stuck at zero alongside the footer budget.
        let (acc_in, acc_out) = if tokens_in > 0 || tokens_out > 0 {
            (tokens_in, tokens_out)
        } else {
            // Endpoint omitted usage: estimate THIS turn's input as the prompt we
            // sent (the whole messages array) and output as the assistant reply —
            // NOT the accumulated session total, which would double-count every
            // prior turn and trip --max-session-tokens after 1-2 turns on
            // usage-less endpoints.
            (
                estimate_messages_tokens(&messages),
                estimate_message_tokens(&assistant_msg),
            )
        };
        *st.tokens_in.lock().await += acc_in;
        *st.tokens_out.lock().await += acc_out;
        // Accumulate prefix-cache hits so /stats can show cache effectiveness.
        *st.cached_tokens.lock().await += cached_tokens;

        // Append + persist the finalized assistant message.
        {
            messages.push(assistant_msg.clone());
            let mut conv = st.conversation.lock().await;
            conv.clone_from(&messages);
            if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                session::append(p, conv.last().unwrap());
            }
        }
        *st.estimated_tokens.lock().await += estimate_message_tokens(&assistant_msg);

        let tool_calls = assistant_msg.tool_calls().map(|tc| tc.to_vec());
        match tool_calls {
            Some(calls) if !calls.is_empty() => {
                // Leading contiguous readonly recon tools (≥2) run as a parallel
                // wave after per-call gates. Writes / finish / bash / … stay in
                // the sequential loop below so HITL and side effects stay ordered.
                let mut call_offset = 0usize;
                {
                    let mut wave_end = 0usize;
                    while wave_end < calls.len()
                        && tools::is_parallel_wave_tool(&calls[wave_end].function.name)
                    {
                        wave_end += 1;
                    }
                    if wave_end >= 2 {
                        match run_parallel_readonly_wave(
                            st,
                            &run,
                            &calls[..wave_end],
                            &tool_defs,
                            &cancel,
                            &mut turn_tool_calls,
                            &mut shape_tools,
                            &mut shape_files,
                        )
                        .await
                        {
                            ParallelWaveResult::Aborted => return,
                            ParallelWaveResult::Done => call_offset = wave_end,
                        }
                    }
                }
                for tc in &calls[call_offset..] {
                    // Honor an abort mid-batch: without this, the synchronous
                    // fall-through tools (write_file/edit/patch/read_file/…)
                    // run to completion after the user hit /abort — only
                    // bash/fetch/web_search/diagnostics were cancel-wrapped.
                    // Check before each call so a batch's remaining
                    // destructive writes don't execute once the turn is
                    // cancelled. (Any orphaned tool_calls this leaves are
                    // repaired by the always-run sanitizer next turn.)
                    if cancel.is_cancelled() {
                        sync_session_file(st).await;
                        emit_aborted_done();
                        return;
                    }
                    let id = tc.id.clone();
                    let name = tc.function.name.clone();
                    let args_str = tc.function.arguments.clone();
                    emit(
                        &Event::new("tool_call")
                            .with("id", json!(id))
                            .with("name", json!(name))
                            .with("args", json!(args_str)),
                    );
                    // Accumulate the turn's work-shape for auto-reflect (skip
                    // `finish` — it signals completion, not work).
                    if name != "finish" {
                        turn_tool_calls = turn_tool_calls.saturating_add(1);
                        shape_tools.push(name.clone());
                        for cat in extract_file_categories(&name, &args_str) {
                            shape_files.push(cat);
                        }
                    }
                    let args: Value = match serde_json::from_str(&args_str) {
                        Ok(v) => v,
                        Err(_) => {
                            // Malformed JSON arguments: the model produced an argument
                            // string that isn't valid JSON (common with long, quote-heavy
                            // commands wrapped inside `bulk`'s nested JSON). Return an
                            // actionable error so the model retries simply, and flag the
                            // conversation for argument sanitization so the malformed
                            // assistant message doesn't make the next API request fail
                            // with "function.arguments must be valid JSON" — which would
                            // repeat on every turn and brick the session.
                            let msg = format!(
                                "tool call '{}' produced malformed JSON arguments (the argument string was not valid JSON). This usually happens with long, quote-heavy commands wrapped inside bulk's nested JSON. Re-issue it simply: call bash directly (not via bulk), and for complex logic write a script to a file with write_file then run `bash script.sh` instead of inlining one long command string.",
                                name
                            );
                            emit(
                                &Event::new("tool_result")
                                    .with("id", json!(id))
                                    .with("ok", json!(false))
                                    .with("output", json!(msg)),
                            );
                            let tool_result = Message::tool(id.clone(), msg);
                            let est = estimate_message_tokens(&tool_result);
                            let mut conv = st.conversation.lock().await;
                            conv.push(tool_result);
                            if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                                session::append(p, conv.last().unwrap());
                            }
                            *st.estimated_tokens.lock().await += est;
                            continue;
                        }
                    };

                    // Tool-schema gate: only tools currently offered in `tool_defs`
                    // may run. Deferred tools (git_*, fetch, bulk_*, …) stay out of
                    // the schema until `load_tools` enables them — without this
                    // gate a model that invents the name (e.g. after reading a
                    // skill) would still execute and defeat staging.
                    let offered = tool_defs.iter().any(|d| {
                        d.get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|v| v.as_str())
                            == Some(name.as_str())
                    });
                    if !offered {
                        let msg = if tools::is_deferred_tool(&name) {
                            format!(
                                "tool '{name}' is deferred and not enabled this session. \
                                 Call load_tools with tools:[\"{name}\"] (or a group: git, web, bulk, browser, all), \
                                 then retry the call."
                            )
                        } else {
                            format!(
                                "tool '{name}' is not available on this agent (not in the current tool list)."
                            )
                        };
                        emit(
                            &Event::new("tool_result")
                                .with("id", json!(id))
                                .with("ok", json!(false))
                                .with("output", json!(msg)),
                        );
                        let tool_result = Message::tool(id.clone(), msg);
                        let est = estimate_message_tokens(&tool_result);
                        let mut conv = st.conversation.lock().await;
                        conv.push(tool_result);
                        if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                            session::append(p, conv.last().unwrap());
                        }
                        *st.estimated_tokens.lock().await += est;
                        continue;
                    }

                    // Approval gate for destructive tools. A plugin tool that
                    // dispatches (a custom name, OR an `override:true` tool that
                    // replaces a built-in) carries its own kind; everything else
                    // uses the static classify() table. A plugin tool that merely
                    // collides with a built-in name (no override) does NOT
                    // dispatch to the plugin, so it falls through to classify().
                    let cfg = st.cfg.read().await.clone();
                    let tool_context = crate::tooling::ToolExecutionContext::new(
                        run.clone(),
                        id.clone(),
                        cfg.clone(),
                        st.runtime.clone(),
                        None,
                    );
                    let Some(_tool_resource) =
                        tool_context.register_resource(ResourceKind::Task, format!("tool:{name}"))
                    else {
                        tool_context.note_stale_result();
                        return;
                    };
                    let kind = match st.plugin_manager.tool_config(&name) {
                        Some(tc) if tc.override_builtin || !tools::is_builtin(&name) => tc.kind,
                        _ => tools::classify(&name),
                    };
                    let kind_str: &'static str = match kind {
                        tools::ToolKind::ReadOnly => "readonly",
                        tools::ToolKind::Destructive => "destructive",
                    };
                    // Skip the gate if the user previously said "always" to this kind.
                    let escalated = st.escalated_kinds.lock().await.contains(kind_str);

                    // Check permission rules before the approval gate.
                    // DENY rules take precedence; ALLOW rules skip the gate entirely.
                    let mut force_allow = false;
                    let mut force_deny = false;
                    let mut force_ask = false;
                    for rule in &cfg.allow_rules {
                        if tool_matches_rule(&name, &args, rule) {
                            force_allow = true;
                            break;
                        }
                    }
                    if !force_allow {
                        for rule in &cfg.deny_rules {
                            if tool_matches_rule(&name, &args, rule) {
                                force_deny = true;
                                break;
                            }
                        }
                    }
                    if !force_allow && !force_deny {
                        for rule in &cfg.ask_rules {
                            if tool_matches_rule(&name, &args, rule) {
                                force_ask = true;
                                break;
                            }
                        }
                    }

                    if force_deny {
                        let msg = format!("tool call '{}' denied by permission rule", name);
                        emit(
                            &Event::new("tool_result")
                                .with("id", json!(id))
                                .with("ok", json!(false))
                                .with("output", json!(msg)),
                        );
                        let tool_result = Message::tool(id.clone(), msg);
                        let est = estimate_message_tokens(&tool_result);
                        let mut conv = st.conversation.lock().await;
                        conv.push(tool_result);
                        if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                            session::append(p, conv.last().unwrap());
                        }
                        *st.estimated_tokens.lock().await += est;
                        continue;
                    }

                    // Restricted ("dangerous") paths (.env, .git/**, .ssh/**, id_rsa, …).
                    // Under `Never` ALL file restrictions are disabled — no
                    // prompt, no block. Under `Destructive`/`Always` a
                    // restricted path forces an approval prompt (for reads AND
                    // writes) instead of the old unconditional hard block; an
                    // approved call proceeds.
                    let restricted = if matches!(cfg.approval, Approval::Never) {
                        None
                    } else {
                        restricted_path_for_tool(&name, &args, &cfg.workspace)
                    };
                    let needs_approval = crate::tooling::approval::approval_required(
                        &cfg.approval,
                        kind,
                        restricted.is_some(),
                        force_allow,
                        escalated,
                        force_ask,
                    );
                    if needs_approval {
                        match request_approval(st, &id, &name, &args_str, kind_str, None, &cancel)
                            .await
                        {
                            ApprovalResult::Granted => {}
                            ApprovalResult::Denied => {
                                let msg = format!("tool call '{}' was denied by the user", name);
                                emit(
                                    &Event::new("tool_result")
                                        .with("id", json!(id))
                                        .with("ok", json!(false))
                                        .with("output", json!(msg)),
                                );
                                let tool_result = Message::tool(id.clone(), msg);
                                let est = estimate_message_tokens(&tool_result);
                                let mut conv = st.conversation.lock().await;
                                conv.push(tool_result);
                                if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                                    session::append(p, conv.last().unwrap());
                                }
                                *st.estimated_tokens.lock().await += est;
                                continue;
                            }
                            ApprovalResult::Aborted => {
                                sync_session_file(st).await;
                                emit(&Event::new("aborted"));
                                emit(&Event::new("done"));
                                return;
                            }
                        }
                    }

                    // Auto-checkpoint before the first destructive mutation in a
                    // turn so Undo can restore the filesystem as well as chat.
                    if kind == tools::ToolKind::Destructive {
                        maybe_auto_checkpoint(st).await;
                    }

                    // Dispatch pre-execution hooks for this tool. Two phases compose:
                    //   1. the tool-SPECIFIC pre_* hook (pre_bash/pre_write/pre_read)
                    //      — transforms/audits/denies that tool's call; and
                    //   2. the catch-all `pre_tool` hook, which fires for EVERY tool
                    //      (memory, todo_write, git_*, subagent, plugin tools, …)
                    //      so a plugin can intercept any call — the same reach a
                    //      core edit of this dispatch loop has. pre_tool runs AFTER
                    //      the specific hook so it sees the final amended args.
                    // Each hook may allow (optionally overriding arg fields via
                    // `modify`, and/or posting a `reason`), or deny (the call is
                    // skipped and the reason is returned to the model). Hooks
                    // compose: each sees the args as amended by earlier hooks.
                    let hook_name = match name.as_str() {
                        "bash" => "pre_bash",
                        "write_file" | "edit" => "pre_write",
                        "read_file" | "grep" | "glob" => "pre_read",
                        _ => "",
                    };
                    let any_pre = (!hook_name.is_empty() && st.plugin_manager.has_hook(hook_name))
                        || st.plugin_manager.has_hook("pre_tool");
                    // exec_args starts as the original args and is amended in place
                    // by pre-hooks. Only clone when a hook will actually run, so
                    // large write payloads aren't copied in the common case.
                    let mut exec_args = if any_pre { args.clone() } else { args };
                    let mut hook_notes: Vec<String> = Vec::new();
                    let mut denied: Option<String> = None;
                    if !hook_name.is_empty() {
                        denied = run_pre_hooks(
                            st,
                            &cfg,
                            hook_name,
                            &name,
                            &mut exec_args,
                            &mut hook_notes,
                        )
                        .await;
                    }
                    if denied.is_none() && name != "finish" {
                        denied = run_pre_hooks(
                            st,
                            &cfg,
                            "pre_tool",
                            &name,
                            &mut exec_args,
                            &mut hook_notes,
                        )
                        .await;
                    }
                    if let Some(msg) = denied {
                        emit(
                            &Event::new("tool_result")
                                .with("id", json!(id))
                                .with("ok", json!(false))
                                .with("output", json!(msg)),
                        );
                        let tool_result = Message::tool(id.clone(), msg);
                        let est = estimate_message_tokens(&tool_result);
                        let mut conv = st.conversation.lock().await;
                        conv.push(tool_result);
                        if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                            session::append(p, conv.last().unwrap());
                        }
                        *st.estimated_tokens.lock().await += est;
                        continue;
                    }

                    // bulk inner-call gate: run the same permission deny-rules +
                    // dangerous-path + plugin pre-hook gate on EACH inner call so
                    // destructive ops can't evade the safety floor by hiding inside
                    // a single `bulk` call (the outer deny/hook loop above only sees
                    // the `bulk` call itself). Denied inner calls are recorded by
                    // index and rendered by execute_bulk.
                    let mut bulk_denied: std::collections::HashMap<usize, String> =
                        std::collections::HashMap::new();
                    if name == "bulk" {
                        if let Some(calls) =
                            exec_args.get_mut("calls").and_then(|v| v.as_array_mut())
                        {
                            for (i, c) in calls.iter_mut().enumerate() {
                                let iname = c
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let iargs = c.get("args").cloned().unwrap_or(json!({}));
                                let mut modified = iargs.clone();
                                let mut dmsg: Option<String> = None;
                                // Deferred-tool staging: bulk must not smuggle
                                // fetch/git_*/web_search/… that aren't enabled.
                                if tools::is_deferred_tool(&iname)
                                    && !matches!(
                                        iname.as_str(),
                                        "bulk" | "bulk_read" | "bulk_write" | "bulk_edit"
                                    )
                                {
                                    let enabled = st.enabled_deferred_tools.lock().await.clone();
                                    let planning =
                                        st.goal.lock().await.phase == goal::GoalPhase::Planning;
                                    if !(enabled.contains(&iname)
                                        || (iname == "goal_write_plan" && planning))
                                    {
                                        dmsg = Some(format!(
                                            "deferred tool '{iname}' is not enabled — call load_tools first, then retry"
                                        ));
                                    }
                                }
                                // permission deny-rules (ALLOW skips, DENY blocks)
                                let mut force_allow = false;
                                if dmsg.is_none() {
                                    for rule in &cfg.allow_rules {
                                        if tool_matches_rule(&iname, &iargs, rule) {
                                            force_allow = true;
                                            break;
                                        }
                                    }
                                }
                                if dmsg.is_none() && !force_allow {
                                    for rule in &cfg.deny_rules {
                                        if tool_matches_rule(&iname, &iargs, rule) {
                                            dmsg = Some("denied by permission rule".into());
                                            break;
                                        }
                                    }
                                }
                                // plugin pre-hooks (the security-relevant ones)
                                if dmsg.is_none() {
                                    let hook_name = match iname.as_str() {
                                        "bash" => "pre_bash",
                                        "write_file" | "edit" => "pre_write",
                                        "read_file" | "grep" | "glob" => "pre_read",
                                        _ => "",
                                    };
                                    if !hook_name.is_empty() {
                                        if let Some(deny) = run_pre_hooks(
                                            st,
                                            &cfg,
                                            hook_name,
                                            &iname,
                                            &mut modified,
                                            &mut hook_notes,
                                        )
                                        .await
                                        {
                                            dmsg = Some(deny);
                                        }
                                    }
                                    if dmsg.is_none() && iname != "finish" {
                                        if let Some(deny) = run_pre_hooks(
                                            st,
                                            &cfg,
                                            "pre_tool",
                                            &iname,
                                            &mut modified,
                                            &mut hook_notes,
                                        )
                                        .await
                                        {
                                            dmsg = Some(deny);
                                        }
                                    }
                                }
                                if let Some(m) = dmsg {
                                    bulk_denied.insert(i, m);
                                } else {
                                    *c = json!({ "name": iname, "args": modified });
                                }
                            }
                        }
                    }

                    // Execute. bash/bulk/diagnostics/spawn are async; others sync.
                    // The async ones are wrapped in a `select!` on the turn cancel
                    // so /abort can interrupt them mid-flight — kill_on_drop frees
                    // the spawned child when the future is dropped.
                    tool_context.persist_state(session::RunState::Started, Some(name.as_str()));
                    let mut outcome = if let Some(restored) = {
                        // Identical re-call of a read-only tool after a digest /
                        // ingress-cap: restore content without re-executing.
                        // Restore is capped so a re-call cannot re-bloat context.
                        let cache = st.tool_output_cache.lock().await;
                        cache.get(&name, &args_str).map(|s| s.to_string())
                    } {
                        tools::Outcome::ok(apply_restore_cap(&restored))
                    } else if let Some((prior_id, preview)) = {
                        let conv = st.conversation.lock().await;
                        find_duplicate_tool_result(&conv, &name, &args_str)
                    } {
                        // Same tool+args already in history undigested — point at
                        // it instead of duplicating tens of KB in the transcript.
                        tools::Outcome::ok(format!(
                            "[duplicate of tool_call_id {prior_id}; content unchanged]\n{preview}"
                        ))
                    } else if let Some(tc) = st
                        .plugin_manager
                        .tool_config(&name)
                        .filter(|tc| tc.override_builtin || !tools::is_builtin(&name))
                    {
                        // Plugin-declared tool: dispatch to its handler script
                        // (subprocess, stdin=args JSON, stdout={ok,output}).
                        // This branch covers BOTH custom plugin tools (a name no
                        // built-in owns) AND `override:true` tools that REPLACE
                        // a built-in's implementation — the filter admits a
                        // built-in name only when the plugin explicitly opted
                        // into overriding it, so a mere name collision still
                        // falls through to the built-in handler below. Wrapped in
                        // a select! on the turn cancel so /abort can interrupt it
                        // mid-flight; kill_on_drop frees the child.
                        let session_id = cfg
                            .session_file
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        let ws = cfg.workspace.display().to_string();
                        tokio::select! {
                            o = plugins::execute_plugin_tool(&name, &tc, &exec_args, &ws, &session_id) => o,
                            _ = cancel.cancelled() => tools::Outcome::err(format!("{name} aborted")),
                        }
                    } else if name == "bash" {
                        let cmd = exec_args
                            .get("command")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let timeout_override = exec_args.get("timeout").and_then(|v| v.as_u64());
                        // Sudo passthrough: if the command invokes `sudo`, we
                        // must NOT let it run directly — sudo opens /dev/tty to
                        // read the password, which garbles the TUI. Instead we
                        // surface a `sudo_request` to the user. On approve the
                        // password is fed via `sudo -S` on stdin; on decline
                        // (Esc) the agent is told the user declined.
                        //
                        // In Approval::Never, probe sudo non-interactively and
                        // prompt only when it explicitly asks for a password.
                        // NOPASSWD/cached credentials run immediately; other
                        // failures are surfaced by `sudo -n` without a flyout.
                        if tools::command_uses_sudo(cmd) {
                            let needs_prompt = if matches!(cfg.approval, Approval::Never) {
                                let sudo_preflight = tools::sudo_preflight(&cfg).await;
                                tools::sudo_should_prompt(&cfg.approval, sudo_preflight)
                            } else {
                                true
                            };
                            if needs_prompt {
                                match request_sudo(st, cmd, &cancel).await {
                                    SudoResult::Approved { password } => {
                                        tokio::select! {
                                            o = tools::execute_bash(cmd, &cfg, timeout_override, tools::SudoAuth::Password(password)) => o,
                                            _ = cancel.cancelled() => tools::Outcome::err("bash aborted"),
                                        }
                                    }
                                    SudoResult::Declined => tools::Outcome::ok(
                                        "The user declined the sudo request — the \
                                         command was NOT run. Ask the user to run it \
                                         manually, or re-attempt without sudo.",
                                    ),
                                    SudoResult::Aborted => {
                                        sync_session_file(st).await;
                                        emit(&Event::new("aborted"));
                                        emit(&Event::new("done"));
                                        return;
                                    }
                                }
                            } else {
                                // NOPASSWD / cached — run with `sudo -n`
                                // (non-interactive, never opens /dev/tty).
                                tokio::select! {
                                    o = tools::execute_bash(cmd, &cfg, timeout_override, tools::SudoAuth::NonInteractive) => o,
                                    _ = cancel.cancelled() => tools::Outcome::err("bash aborted"),
                                }
                            }
                        } else {
                            tokio::select! {
                                o = tools::execute_bash(cmd, &cfg, timeout_override, tools::SudoAuth::None) => o,
                                _ = cancel.cancelled() => tools::Outcome::err("bash aborted"),
                            }
                        }
                    } else if name == "bulk" {
                        tokio::select! {
                            o = tools::execute_bulk(&exec_args, &cfg, &bulk_denied) => o,
                            _ = cancel.cancelled() => tools::Outcome::err("bulk aborted"),
                        }
                    } else if name == "fetch" {
                        tokio::select! {
                            o = tools::execute_fetch(&exec_args, &cfg) => o,
                            _ = cancel.cancelled() => tools::Outcome::err("fetch aborted"),
                        }
                    } else if name == "web_search" {
                        tokio::select! {
                            o = tools::execute_web_search(&exec_args, &cfg) => o,
                            _ = cancel.cancelled() => tools::Outcome::err("web_search aborted"),
                        }
                    } else if name == "diagnostics" {
                        tokio::select! {
                            o = tools::execute_diagnostics(&exec_args, &cfg) => o,
                            _ = cancel.cancelled() => tools::Outcome::err("diagnostics aborted"),
                        }
                    } else if name == "test_env" {
                        tokio::select! {
                            o = tools::execute_test_env(&exec_args, &cfg) => o,
                            _ = cancel.cancelled() => tools::Outcome::err("test_env aborted"),
                        }
                    } else if browser::is_browser_tool(&name) {
                        tokio::select! {
                            o = browser::execute_browser(&name, &exec_args, &cfg) => o,
                            _ = cancel.cancelled() => tools::Outcome::err("browser tool aborted"),
                        }
                    } else if name == "spawn" || name == "subagent" {
                        // When goal mode is active, cap concurrency on parallel
                        // fan-out to the goal's limit (defense in depth).
                        let mut sub_args = exec_args.clone();
                        {
                            let g = st.goal.lock().await;
                            if g.is_active() {
                                if let Some(c) =
                                    sub_args.get("concurrency").and_then(|v| v.as_u64())
                                {
                                    sub_args["concurrency"] =
                                        json!(goal::cap_concurrency(c as u32, &g));
                                } else if sub_args.get("tasks").is_some() {
                                    sub_args["concurrency"] = json!(g.concurrency);
                                }
                            }
                        }
                        subagent::execute(
                            st.clone(),
                            client.clone(),
                            provider.clone(),
                            model.clone(),
                            sub_args,
                            cancel.clone(),
                            0,
                        )
                        .await
                    } else if name == "goal_write_plan" {
                        goal::handle_goal_write_plan(st, &exec_args).await
                    } else if name == "load_tools" {
                        handle_load_tools(st, &exec_args, &mut tool_defs).await
                    } else if name == "ask" {
                        // CEO / Control Center autonomy: never block on the user.
                        let ceo_active = {
                            let g = st.goal.lock().await;
                            g.ceo_mode && g.is_active()
                        };
                        if ceo_active {
                            tools::Outcome::ok(
                                "CEO mode is active — do not ask the user. Decide autonomously,                                  document assumptions, and continue. Do not re-call ask.".to_string(),
                            )
                        } else {
                            // Blocking user-interaction tool: surface a flyout and
                            // wait for the answer (or skip/abort). Validation errors
                            // and skips return a normal Outcome; an abort ends the
                            // turn like the approval gate does.
                            match request_ask(st, &exec_args, &cancel).await {
                            AskResult::Answered { questions, answers } => {
                                tools::Outcome::ok(format_ask_answers(&questions, &answers))
                            }
                            AskResult::Skipped => tools::Outcome::ok(
                                "The user skipped the questions. Proceed with your best judgment and note any assumptions.",
                            ),
                            AskResult::Aborted => {
                                sync_session_file(st).await;
                                emit(&Event::new("aborted"));
                                emit(&Event::new("done"));
                                return;
                            }
                        }
                        }
                    } else if name == "memory" {
                        // Route through the plugin memory_provider when one is
                        // active and no tool override already handled this call.
                        if let Some(mp) = st.plugin_manager.memory_provider() {
                            let action = exec_args
                                .get("action")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let session_id = cfg
                                .session_file
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default();
                            let ws = cfg.workspace.display().to_string();
                            tokio::select! {
                                r = plugins::execute_memory_provider(&mp, action, &exec_args, &ws, &session_id) => r.into_outcome(),
                                _ = cancel.cancelled() => tools::Outcome::err("memory aborted"),
                            }
                        } else {
                            let n = name.clone();
                            let a = exec_args.clone();
                            let c = cfg.clone();
                            match tokio::task::spawn_blocking(move || tools::execute(&n, &a, &c))
                                .await
                            {
                                Ok(o) => o,
                                Err(_) => tools::Outcome::err("tool task panicked"),
                            }
                        }
                    } else {
                        let n = name.clone();
                        let a = exec_args.clone();
                        let c = cfg.clone();
                        match tokio::task::spawn_blocking(move || tools::execute(&n, &a, &c)).await
                        {
                            Ok(o) => o,
                            Err(_) => tools::Outcome::err("tool task panicked"),
                        }
                    };

                    // Execution may have completed concurrently with abort,
                    // steering, or session replacement. Never apply that late
                    // result to conversation/work state; the lifecycle command
                    // owns the sole terminal event for the cancelled run.
                    if !tool_context.is_active() {
                        tool_context.note_stale_result();
                        return;
                    }

                    // Milestone 1.1: a memory save/append/forget via the AI
                    // tool must be visible to subsequent turns in THIS session,
                    // so rebuild the memory slice of the system prompt now (no-op
                    // + prefix-cache-safe when unchanged). The /memory,
                    // /save-memory and /forget-memory commands refresh from their
                    // own handlers; this covers the model's tool path.
                    if name == "memory" {
                        let action = exec_args
                            .get("action")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if matches!(action, "save" | "append" | "forget" | "consolidate") {
                            refresh_memory_injection(st).await;
                        }
                    }

                    // Rolling work-state: mirror todo_write + file edits into the
                    // KV-cache-aware summary so the model sees current work state
                    // every turn without a tool call. Only on success so a failed
                    // write doesn't pollute the recent-files list.
                    if outcome.ok {
                        match name.as_str() {
                            "todo_write" => sync_work_state_from_todos(st, &exec_args).await,
                            "write_file" | "edit" | "patch" | "bulk_write" | "bulk_edit"
                            | "delete" | "rename" | "mkdir" => {
                                record_file_touch(st, &name, &exec_args).await
                            }
                            "read_file" => {
                                if let Some(path) = exec_args.get("path").and_then(|v| v.as_str()) {
                                    let lower = path.to_ascii_lowercase();
                                    if lower.ends_with("skill.md")
                                        || lower.contains("/.catalyst-code/skills/")
                                        || lower.contains("\\.catalyst-code\\skills\\")
                                    {
                                        st.skill_read_count
                                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }

                    // Dispatch post-execution hooks for this tool. Two phases,
                    // mirroring the pre-hook structure: the tool-SPECIFIC post_*
                    // hook (post_bash/post_write/post_read), then the catch-all
                    // `post_tool` that fires for EVERY tool. Each hook receives the
                    // tool's CURRENT result and may MODIFY it (return
                    // `modify: {"output":…, "ok":…, "diff":…}`) — e.g. redact a
                    // secret, append context, reformat. Post-hooks never block (the
                    // op already ran); `allow:false` is ignored, only `reason` +
                    // `modify` are honored.
                    let post_hook = match name.as_str() {
                        "bash" => "post_bash",
                        "write_file" | "edit" => "post_write",
                        "read_file" | "grep" | "glob" => "post_read",
                        _ => "",
                    };
                    if !post_hook.is_empty() {
                        run_post_hooks(
                            st,
                            &cfg,
                            post_hook,
                            &name,
                            &exec_args,
                            &mut outcome,
                            &mut hook_notes,
                        )
                        .await;
                    }
                    if name != "finish" {
                        run_post_hooks(
                            st,
                            &cfg,
                            "post_tool",
                            &name,
                            &exec_args,
                            &mut outcome,
                            &mut hook_notes,
                        )
                        .await;
                    }
                    let persisted_tool_state = if cancel.is_cancelled() {
                        session::RunState::Cancelled
                    } else if outcome.ok {
                        session::RunState::Completed
                    } else {
                        session::RunState::Failed
                    };
                    tool_context.persist_state(persisted_tool_state, Some(name.as_str()));

                    // finish sentinel: the model signaled completion.
                    if name == "finish" && outcome.ok && outcome.output == tools::FINISH_SENTINEL {
                        // Auto-reflect gate: before the first `finish` exits a
                        // non-trivial turn, inject a reflection continuation (the
                        // deterministic seam SELF_LEARNING §11 deferred) instead
                        // of completing. Falls through to the normal tool-result
                        // push + re-stream; `reflected` prevents re-entry.
                        let mut do_reflect = false;
                        let mut recurrence = 0usize;
                        if !reflected {
                            if let Some((nudge, rec)) = maybe_reflect_prompt(
                                st,
                                &prompt,
                                turn_tool_calls,
                                &shape_tools,
                                &shape_files,
                                cancel.is_cancelled(),
                            )
                            .await
                            {
                                reflected = true;
                                outcome.output = nudge;
                                recurrence = rec;
                                do_reflect = true;
                            }
                        }
                        if do_reflect {
                            emit(&Event::new("reflecting").with("recurrence", json!(recurrence)));
                            // Fall through → the finish tool_result (carrying
                            // the nudge) is pushed below and the loop re-streams.
                        } else {
                            // Emit a real tool_result so the TUI/web don't leave
                            // the finish card empty / stuck as "[no result]".
                            let finish_msg = tools::FINISH_MESSAGE;
                            emit(
                                &Event::new("tool_result")
                                    .with("id", json!(id))
                                    .with("ok", json!(true))
                                    .with("output", json!(finish_msg)),
                            );
                            let tool_result = Message::tool(id.clone(), finish_msg);
                            let est = estimate_message_tokens(&tool_result);
                            messages.push(tool_result.clone());
                            {
                                let mut conv = st.conversation.lock().await;
                                // Keep conversation identical to the working
                                // buffer (clone_from, not push) so a divergent
                                // mid-turn sync can't leave finish unpaired.
                                conv.clone_from(&messages);
                                if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                                    session::append(p, conv.last().unwrap());
                                }
                            }
                            *st.estimated_tokens.lock().await += est;
                            *st.last_turn_time.lock().await = std::time::Instant::now();
                            let (r_in, r_out) = reported_tokens(st, tokens_in, tokens_out).await;
                            let metrics = timer.finalize(r_in, r_out, cached_tokens, model.clone());
                            *st.last_turn_metrics.lock().await = Some(metrics.clone());
                            emit_turn_metrics(st, &metrics).await;
                            st.logger.log("turn_done", json!({ "model": metrics.model, "tokens_in": metrics.tokens_in, "tokens_out": metrics.tokens_out, "cached_tokens": metrics.cached_tokens, "ttft_ms": metrics.ttft_ms, "elapsed_ms": metrics.elapsed_ms, "tps": metrics.tps, "finish_tool": true }));
                            st.logger.record_turn();
                            persist_stats(st).await;
                            sync_session_file(st).await;
                            maybe_finish_goal_orchestrator_turn(st, client, cancel.is_cancelled())
                                .await;
                            emit(&Event::new("done"));
                            return;
                        }
                    }
                    // Surface plugin hook feedback to the model. Any pre-hook that
                    // modified args or posted a reason, and any post-hook that
                    // observed something, is appended to the tool result so the
                    // model knows its write/edit/read/bash call was inspected.
                    if !hook_notes.is_empty() {
                        outcome.output.push_str("\n\nPlugin hooks:\n- ");
                        outcome.output.push_str(&hook_notes.join("\n- "));
                    }
                    // Cross-session anomaly nudge: if another session is
                    // active in this workspace and this tool failed (or touched
                    // a file a peer is editing), append a note so the agent
                    // checks the neighbors before assuming it caused the error.
                    // Uses the cached peer snapshot — no filesystem read here.
                    if let Some(note) =
                        maybe_concurrency_note(st, &name, &exec_args, outcome.ok).await
                    {
                        outcome.output.push_str("\n\n");
                        outcome.output.push_str(&note);
                    }
                    // Cache + ingress: store full output for restorable tools so
                    // digests / caps can shrink context; identical re-calls restore.
                    // Destructive tools wipe the cache (tree may have changed).
                    if outcome.ok {
                        if tool_cache::invalidates_cache(&name) {
                            st.tool_output_cache.lock().await.invalidate_all();
                        } else if tool_cache::ToolOutputCache::is_restorable(&name)
                            && !outcome.output.starts_with("[restored from digest cache]")
                            && !outcome.output.starts_with("[duplicate of tool_call_id")
                        {
                            st.tool_output_cache.lock().await.store(
                                &name,
                                &args_str,
                                &outcome.output,
                            );
                        }
                    }
                    // Hard ingress cap: never let a single tool result dominate
                    // the context window. Prefer smart-truncation over an opaque
                    // digest on first ingress; soft digest collapses further later.
                    // Skip restore/duplicate pointers (already compact).
                    if outcome.ok
                        && !outcome.output.starts_with("[restored from digest cache]")
                        && !outcome.output.starts_with("[duplicate of tool_call_id")
                    {
                        outcome.output = apply_ingress_cap(&name, &args_str, outcome.output);
                    }
                    // Keep observability useful without copying commands, file contents,
                    // or credentials into the debug log. The stable hash correlates this
                    // record with the separately redacted audit trail.
                    let status =
                        crate::tooling::ToolResultStatus::from_legacy(outcome.ok, &outcome.output);
                    st.logger.log(
                        "tool",
                        json!({
                            "tool_call_id": &id,
                            "name": &name,
                            "args_hash": audit::args_hash(&args_str),
                            "status": status.as_str(),
                            "output_len": outcome.output.len(),
                            "duration_ms": tool_context.elapsed_ms(),
                        }),
                    );
                    let mut ev = Event::new("tool_result")
                        .with("id", json!(id))
                        .with("ok", json!(outcome.ok))
                        .with("output", json!(outcome.output));
                    // Surface a unified-diff rendering to the TUI as a separate
                    // `diff` field (edit/patch/write_file). It is NOT added to the
                    // model-facing tool content (`output`) so the model's context
                    // stays compact — the diff is for the human approver.
                    if let Some(d) = &outcome.diff {
                        ev = ev.with("diff", json!(d));
                    }
                    emit(&ev);
                    if outcome.ok {
                        if let Some(d) = &outcome.diff {
                            if !d.is_empty() {
                                let path =
                                    exec_args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                                emit(
                                    &Event::new("file_change")
                                        .with("path", json!(path))
                                        .with("unified_diff", json!(d))
                                        .with("tool", json!(name)),
                                );
                            }
                        } else if matches!(
                            name.as_str(),
                            "write_file" | "edit" | "patch" | "delete" | "rename" | "mkdir"
                        ) {
                            let path = exec_args
                                .get("path")
                                .or_else(|| exec_args.get("to"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if !path.is_empty() {
                                emit(
                                    &Event::new("file_change")
                                        .with("path", json!(path))
                                        .with("tool", json!(name)),
                                );
                            }
                        }
                    }
                    let tool_result = Message::tool(id.clone(), &outcome.output);
                    let est = estimate_message_tokens(&tool_result);
                    messages.push(tool_result.clone());
                    let mut conv = st.conversation.lock().await;
                    conv.push(tool_result);
                    if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                        session::append(p, conv.last().unwrap());
                    }
                    *st.estimated_tokens.lock().await += est;
                }
                // Re-sync after parallel wave / any path that touched conversation
                // without going through the working buffer.
                messages.clone_from(&*st.conversation.lock().await);
                // Loop back for the model to continue.
            }
            _ => {
                // Turn complete — or, on a non-trivial turn, inject a reflect
                // continuation before the real completion (auto-reflect gate).
                let mut do_reflect = false;
                let mut recurrence = 0usize;
                let mut reflect_prompt = String::new();
                if !reflected {
                    if let Some((p, rec)) = maybe_reflect_prompt(
                        st,
                        &prompt,
                        turn_tool_calls,
                        &shape_tools,
                        &shape_files,
                        cancel.is_cancelled(),
                    )
                    .await
                    {
                        reflected = true;
                        reflect_prompt = p;
                        recurrence = rec;
                        do_reflect = true;
                    }
                }
                if do_reflect {
                    // Push the reflect prompt as a user message and re-stream.
                    let msg = Message::user(reflect_prompt);
                    let est = estimate_message_tokens(&msg);
                    messages.push(msg.clone());
                    let mut conv = st.conversation.lock().await;
                    conv.push(msg);
                    if let Some(p) = st.cfg.read().await.session_file.as_ref() {
                        session::append(p, conv.last().unwrap());
                    }
                    *st.estimated_tokens.lock().await += est;
                    drop(conv);
                    emit(&Event::new("reflecting").with("recurrence", json!(recurrence)));
                    // Don't return → the outer loop re-streams the reflection.
                } else {
                    // Turn complete: emit metrics + done.
                    *st.last_turn_time.lock().await = std::time::Instant::now();
                    let (r_in, r_out) = reported_tokens(st, tokens_in, tokens_out).await;
                    let metrics = timer.finalize(r_in, r_out, cached_tokens, model.clone());
                    *st.last_turn_metrics.lock().await = Some(metrics.clone());
                    emit_turn_metrics(st, &metrics).await;
                    st.logger.log("turn_done", json!({ "model": metrics.model, "tokens_in": metrics.tokens_in, "tokens_out": metrics.tokens_out, "cached_tokens": metrics.cached_tokens, "ttft_ms": metrics.ttft_ms, "elapsed_ms": metrics.elapsed_ms, "tps": metrics.tps }));
                    st.logger.record_turn();
                    persist_stats(st).await;
                    maybe_finish_goal_orchestrator_turn(st, client, cancel.is_cancelled()).await;
                    emit(&Event::new("done"));
                    return;
                }
            }
        }
    }
}
