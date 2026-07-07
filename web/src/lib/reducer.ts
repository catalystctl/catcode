// The agent state reducer.
//
// Shared by the server bridge (reduces the full event stream to maintain a
// snapshot for reconnecting clients) and the browser (reduces live SSE events).
// One reducer, one source of truth for the message-assembly rules — mirrors the
// SDK's AgentSession logic: a new assistant message is begun lazily on the first
// delta/thinking; a tool_call finalizes the current assistant (clearing it) so
// the next delta starts a fresh assistant message (multi-step agent turns);
// tool_result matches its tool call by id.

import type {
  AgentEvent,
  AgentState,
  AssistantMsg,
  CoreEvent,
  IntercomEntry,
  SubagentChatItem,
  SubagentRunView,
  Toast,
  UIMessage,
  UIToolCall,
} from "./types";

export const initialState: AgentState = {
  ready: null,
  models: [],
  authed: null,
  provider: "",
  providerKind: "",
  approvalMode: "destructive",
  escalatedKinds: [],
  workspace: "",
  projects: [],
  providerPresets: [],
  selectedModel: null,
  thinkingLevel: "medium",
  messages: [],
  currentAssistantId: null,
  streaming: false,
  retrying: false,
  pendingApproval: null,
  pendingAsk: null,
  metrics: null,
  umansConc: null,
  sessions: [],
  currentSessionFile: null,
  stats: null,
  toasts: [],
  memories: [],
  plugins: [],
  skills: [],
  pendingIntercom: null,
  pendingOauth: null,
  intercomLog: [],
  subagentRuns: {},
  visionConfig: null,
  workState: null,
  switching: false,
};

let counter = 0;
function newId(prefix: string): string {
  counter += 1;
  return `${prefix}_${Date.now().toString(36)}_${counter.toString(36)}`;
}

function pushToast(toasts: Toast[], kind: Toast["kind"], message: string): Toast[] {
  const next = [...toasts, { id: newId("t"), kind, message }];
  // ponytail: keep the last 6 toasts so the stack can't grow unbounded.
  return next.length > 6 ? next.slice(next.length - 6) : next;
}

/** Get-or-create a subagent run view for a run_id, then apply a mutation. Used
 *  by every subagent_* event so ordering gaps (a progress arriving before its
 *  start) never crash — a missing run is created as a running stub. */
function upsertRun(
  state: AgentState,
  runId: string,
  fn: (r: SubagentRunView) => SubagentRunView,
): AgentState {
  const prev: SubagentRunView =
    state.subagentRuns[runId] ?? {
      id: runId,
      mode: "",
      agents: [],
      task: "",
      state: "running",
      depth: 0,
      startedAt: 0,
      toolCount: 0,
      tokensIn: 0,
      tokensOut: 0,
      elapsedMs: 0,
      items: [],
    };
  return {
    ...state,
    subagentRuns: { ...state.subagentRuns, [runId]: fn(prev) },
  };
}

/** Begin a new assistant message if none is in flight. */
function beginAssistant(state: AgentState, model?: string): AgentState {
  if (state.currentAssistantId) return state;
  const id = newId("a");
  const msg: AssistantMsg = {
    id,
    role: "assistant",
    text: "",
    thinking: "",
    toolCalls: [],
    model: model ?? state.selectedModel ?? undefined,
    streaming: true,
    ts: Date.now(),
  };
  return {
    ...state,
    messages: [...state.messages, msg],
    currentAssistantId: id,
    streaming: true,
  };
}

function updateCurrentAssistant(
  state: AgentState,
  fn: (m: AssistantMsg) => AssistantMsg,
): AgentState {
  const id = state.currentAssistantId;
  if (!id) return state;
  return {
    ...state,
    messages: state.messages.map((m) => (m.id === id && m.role === "assistant" ? fn(m) : m)),
  };
}

function finalizeCurrentAssistant(state: AgentState): AgentState {
  const id = state.currentAssistantId;
  if (!id) return state;
  return {
    ...state,
    messages: state.messages.map((m) =>
      m.id === id && m.role === "assistant" ? { ...m, streaming: false } : m,
    ),
    currentAssistantId: null,
  };
}

function finishTurn(state: AgentState): AgentState {
  return {
    ...finalizeCurrentAssistant(state),
    streaming: false,
    retrying: false,
  };
}

function parseArgs(raw: string): { args: Record<string, unknown>; argString: string } {
  const argString = raw ?? "";
  if (!argString) return { args: {}, argString: "" };
  try {
    return { args: JSON.parse(argString) as Record<string, unknown>, argString };
  } catch {
    return { args: { raw: argString }, argString };
  }
}

function onToolCall(
  state: AgentState,
  ev: { id: string; name: string; args: string },
): AgentState {
  const s = beginAssistant(state);
  const id = ev.id || newId("call");
  const { args, argString } = parseArgs(ev.args);
  const tc: UIToolCall = { id, name: ev.name || "", args, argString };
  const withCall = updateCurrentAssistant(s, (m) => ({ ...m, toolCalls: [...m.toolCalls, tc] }));
  // The assistant turn is complete for this message; clear it so the next delta
  // (after the tool result) begins a fresh assistant message.
  return finalizeCurrentAssistant(withCall);
}

function onToolResult(
  state: AgentState,
  ev: { id: string; ok: boolean; output: string; diff?: string; tool?: string },
): AgentState {
  const result = { ok: ev.ok !== false, output: ev.output ?? "", diff: ev.diff };
  let matched = false;
  const messages = state.messages.map((m) => {
    if (m.role !== "assistant") return m;
    if (!m.toolCalls.some((t) => t.id === ev.id)) return m;
    matched = true;
    return {
      ...m,
      toolCalls: m.toolCalls.map((t) => (t.id === ev.id ? { ...t, result } : t)),
    };
  });
  if (matched) return { ...state, messages };
  // Fallback: no matching tool call (shouldn't happen) — render a standalone card.
  const fallback: UIMessage = {
    id: newId("tool"),
    role: "tool",
    toolCallId: ev.id,
    toolName: ev.tool ?? "",
    output: ev.output ?? "",
    ok: ev.ok !== false,
    diff: ev.diff,
    ts: Date.now(),
  };
  return { ...state, messages: [...state.messages, fallback] };
}

function asText(content: unknown): string {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .map((c: any) =>
        typeof c === "string"
          ? c
          : c?.type === "text"
            ? c.text ?? ""
            : c?.type === "toolcall"
              ? ""
              : c?.text ?? "",
      )
      .join("");
  }
  return "";
}

function asThinking(m: any): string {
  if (typeof m.reasoning_content === "string") return m.reasoning_content;
  if (Array.isArray(m.content)) {
    return m.content
      .filter((c: any) => c?.type === "thinking" || c?.type === "reasoning")
      .map((c: any) => c.thinking ?? c.text ?? "")
      .join("");
  }
  return "";
}

/** Extract image data URLs from a multimodal user message content array. */
function asImages(content: unknown): string[] {
  if (!Array.isArray(content)) return [];
  const out: string[] = [];
  for (const c of content as any[]) {
    if (c?.type === "image_url" && c.image_url?.url) {
      out.push(String(c.image_url.url));
    }
  }
  return out;
}

/** Convert an OpenAI-style history array (from the core's `history` event) into
 *  the UI message model. Tool results attach to their tool call when possible. */
function historyToMessages(raw: unknown[]): UIMessage[] {
  const list = Array.isArray(raw) ? raw : [];
  const out: UIMessage[] = [];
  for (const item of list) {
    const m: any = item;
    if (!m || typeof m !== "object") continue;
    const role = m.role;
    const ts = typeof m.timestamp === "number" ? m.timestamp : Date.now();
    if (role === "user") {
      const imgs = asImages(m.content);
      out.push({ id: newId("u"), role: "user", text: asText(m.content), ts, ...(imgs.length ? { images: imgs } : {}) });
    } else if (role === "assistant") {
      const toolCalls: UIToolCall[] = (Array.isArray(m.tool_calls) ? m.tool_calls : []).map(
        (tc: any) => {
          const fn = tc?.function ?? {};
          const { args, argString } = parseArgs(typeof fn.arguments === "string" ? fn.arguments : "");
          return { id: tc?.id ?? newId("call"), name: fn.name ?? "", args, argString };
        },
      );
      out.push({
        id: newId("a"),
        role: "assistant",
        text: asText(m.content),
        thinking: asThinking(m),
        toolCalls,
        model: m.model,
        streaming: false,
        ts,
      });
    } else if (role === "tool") {
      const tcId = m.tool_call_id ?? "";
      const content = asText(m.content);
      const ownerIdx = out.findIndex(
        (x) => x.role === "assistant" && x.toolCalls.some((t) => t.id === tcId),
      );
      if (ownerIdx >= 0) {
        const a = out[ownerIdx] as AssistantMsg;
        out[ownerIdx] = {
          ...a,
          toolCalls: a.toolCalls.map((t) =>
            // History carries no ok/error flag, so mark the result unknown —
            // the card renders a neutral badge instead of a green “ok”.
            t.id === tcId ? { ...t, result: { ok: true, output: content, unknown: true } } : t,
          ),
        };
      } else {
        out.push({
          id: newId("tool"),
          role: "tool",
          toolCallId: tcId,
          toolName: "",
          output: content,
          ok: true,
          ts,
        });
      }
    }
  }
  return out;
}

export function reduce(state: AgentState, ev: AgentEvent): AgentState {
  switch (ev.type) {
    // ── Synthetic events ──
    case "_user": {
      const msg: UIMessage = {
        id: newId("u"),
        role: "user",
        text: ev.text,
        ts: Date.now(),
        steer: ev.steer,
      };
      return {
        ...state,
        messages: [...state.messages, msg],
        selectedModel: ev.model ?? state.selectedModel,
        streaming: true,
      };
    }
    case "_select_model":
      return { ...state, selectedModel: ev.id };
    case "_set_thinking":
      return { ...state, thinkingLevel: ev.level };
    case "_dismiss_toast":
      return { ...state, toasts: state.toasts.filter((t) => t.id !== ev.id) };
    case "_set_switching":
      return { ...state, switching: ev.switching };
    case "_session_title": {
      const sessions = state.sessions.map((s) =>
        s.name === ev.name ? { ...s, title: ev.title || undefined } : s,
      );
      return { ...state, sessions };
    }

    // ── Core events ──
    case "ready":
      return {
        ...state,
        ready: ev,
        models: ev.models ?? [],
        authed: ev.authed,
        provider: ev.provider,
        providerKind: ev.providerKind,
        approvalMode: ev.approval,
        workspace: ev.workspace,
        providerPresets: ev.providerPresets ?? state.providerPresets,
      };
    case "models":
      return { ...state, models: ev.models ?? [] };
    case "provider_presets":
      return { ...state, providerPresets: ev.presets ?? [] };
    case "authed":
      return { ...state, authed: ev.ok, pendingOauth: null };
    case "provider_changed":
      return { ...state, provider: ev.provider, providerKind: ev.kind, authed: ev.has_key, pendingOauth: null };
    case "approval_changed": {
      if (ev.mode.includes(":")) {
        const kind = ev.mode.split(":")[0];
        return {
          ...state,
          escalatedKinds: state.escalatedKinds.includes(kind)
            ? state.escalatedKinds
            : [...state.escalatedKinds, kind],
        };
      }
      return { ...state, approvalMode: ev.mode };
    }
    case "delta": {
      const s = beginAssistant(state);
      return updateCurrentAssistant(s, (m) => ({ ...m, text: m.text + ev.text }));
    }
    case "thinking": {
      const s = beginAssistant(state);
      return updateCurrentAssistant(s, (m) => ({ ...m, thinking: m.thinking + ev.text }));
    }
    case "tool_call_start":
      // The SDK ignores tool_call_start/name/args deltas — the full card arrives
      // with the finalized `tool_call` event (beginAssistant there). Avoids an
      // empty bubble during args streaming.
      return state;
    case "tool_call_name":
    case "tool_call_args":
      return state;
    case "tool_call":
      return onToolCall(state, ev);
    case "tool_result":
      return onToolResult(state, ev);
    case "umans_conc": {
      const { used, limit, provider } = ev;
      // Live account-wide Umans concurrency from the /v1/usage poll. null used
      // => not Umans / fetch failed (hide); null limit => unlimited (render ∞).
      // `provider` is the Umans provider the poll tracks; the header only shows
      // the field when the selected model routes to it.
      return { ...state, umansConc: { used: used ?? null, limit: limit ?? null, provider: provider ?? "" } };
    }
    case "metrics": {
      const { type: _t, ...rest } = ev;
      const metrics = { ...state.metrics, ...rest };
      // Final metrics (carry elapsed_ms / prompt_tokens): attach usage to the last
      // assistant message for per-message display.
      const isFinal = ev.elapsed_ms != null || ev.prompt_tokens != null;
      let messages = state.messages;
      if (isFinal) {
        const lastA = [...messages].reverse().find((m) => m.role === "assistant");
        if (lastA) {
          messages = messages.map((m) =>
            m.id === lastA.id ? { ...m, usage: metrics ?? undefined } : m,
          );
        }
      }
      return { ...state, metrics, messages };
    }
    case "approval_request":
      return {
        ...state,
        pendingApproval: {
          request_id: ev.request_id,
          tool: ev.tool,
          args: ev.args,
          diff: ev.diff,
        },
      };
    case "ask_request":
      return {
        ...state,
        pendingAsk: { request_id: ev.request_id, questions: ev.questions },
        toasts: pushToast(
          state.toasts,
          "info",
          `Agent asks: ${ev.questions.length} question${ev.questions.length === 1 ? "" : "s"}`,
        ),
      };
    case "compacted":
      return {
        ...state,
        toasts: pushToast(
          state.toasts,
          "info",
          `Context compacted — ${ev.before_tokens.toLocaleString()} → ${ev.after_tokens.toLocaleString()} tokens`,
        ),
      };
    case "http_retry":
      return {
        ...state,
        retrying: true,
        toasts: pushToast(
          state.toasts,
          "info",
          `Retrying request${ev.status ? ` (HTTP ${ev.status})` : ""}…`,
        ),
      };
    case "sessions": {
      const sorted = [...ev.sessions].sort((a, b) => (b.mtime ?? 0) - (a.mtime ?? 0));
      return {
        ...state,
        sessions: sorted,
        currentSessionFile: state.currentSessionFile ?? sorted[0]?.name ?? null,
      };
    }
    case "stats":
      return {
        ...state,
        stats: ev,
        currentSessionFile: ev.session_file || state.currentSessionFile,
      };
    case "history":
      return {
        ...state,
        messages: historyToMessages(ev.messages ?? []),
        currentAssistantId: null,
        streaming: false,
        pendingApproval: null,
        pendingAsk: null,
      };
    case "reset":
      return {
        ...state,
        messages: [],
        currentAssistantId: null,
        streaming: false,
        pendingApproval: null,
        pendingAsk: null,
        workState: null,
      };
    case "done":
      return finishTurn(state);
    case "aborted":
      return finishTurn(state);
    case "error":
      return {
        ...state,
        streaming: false,
        retrying: false,
        toasts: pushToast(state.toasts, "error", ev.message),
      };
    case "info":
      return { ...state, toasts: pushToast(state.toasts, "info", ev.message) };
    case "steer":
      return state;

    // ── Subagent / intercom ──
    case "intercom_message": {
      const msg = ev.message.length > 80 ? ev.message.slice(0, 79) + "…" : ev.message;
      const entry: IntercomEntry = {
        id: newId("ic"),
        kind: ev.reason === "need_decision" || !ev.reason ? "ask" : "reply",
        from: ev.from,
        to: ev.to,
        message: ev.message,
        ts: Date.now(),
      };
      const log = [entry, ...state.intercomLog].slice(0, 50);
      const needsDecision = !ev.reason || ev.reason === "need_decision";
      return {
        ...state,
        intercomLog: log,
        pendingIntercom: needsDecision
          ? { request_id: ev.id, from: ev.from || "subagent", message: ev.message, reason: ev.reason }
          : state.pendingIntercom,
        toasts: needsDecision ? pushToast(state.toasts, "info", `Subagent asks: ${msg}`) : state.toasts,
      };
    }
    case "subagent_progress": {
      // Live phase/tokens/tool counters, keyed by run_id. Also keep a log line
      // (the old reducer read `message`, which the core never emits for progress).
      const phase = ev.phase ?? "";
      const entry: IntercomEntry = {
        id: newId("sp"),
        kind: "status",
        from: ev.agent,
        message: ev.tool ? `${phase}: ${ev.tool}` : phase,
        ts: Date.now(),
      };
      const withLog = { ...state, intercomLog: [entry, ...state.intercomLog].slice(0, 50) };
      return upsertRun(withLog, ev.run_id, (r) => ({
        ...r,
        agent: ev.agent ?? r.agent,
        phase,
        tool: ev.tool ?? r.tool,
        toolCount: ev.tool_count ?? r.toolCount,
        tokensIn: ev.tokens_in ?? r.tokensIn,
        tokensOut: ev.tokens_out ?? r.tokensOut,
        elapsedMs: ev.elapsed_ms ?? r.elapsedMs,
        state: r.state === "running" && phase === "done" ? "completed" : r.state,
      }));
    }
    case "subagent_start":
      return upsertRun(state, ev.run_id, (r) => ({
        ...r,
        mode: ev.mode,
        agent: ev.agent ?? r.agent,
        agents: ev.agents ?? r.agents,
        task: ev.task ?? r.task,
        depth: ev.depth ?? r.depth,
        startedAt: ev.started_at ?? r.startedAt,
        state: "running",
      }));
    case "subagent_message": {
      const item: SubagentChatItem = {
        id: newId("sm"),
        kind: "message",
        role: ev.role === "user" || ev.role === "assistant" ? ev.role : "assistant",
        content: ev.content,
        ts: Date.now(),
      };
      return upsertRun(state, ev.run_id, (r) => ({ ...r, items: [...r.items, item] }));
    }
    case "subagent_tool_call": {
      const item: SubagentChatItem = {
        id: newId("st"),
        kind: "tool",
        callId: ev.call_id,
        name: ev.name,
        args: ev.args,
        ts: Date.now(),
      };
      return upsertRun(state, ev.run_id, (r) => ({
        ...r,
        items: [...r.items, item],
        toolCount: ev.tool_count ?? r.toolCount,
      }));
    }
    case "subagent_tool_result":
      return upsertRun(state, ev.run_id, (r) => {
        // Match by call_id when present; otherwise the most recent tool item
        // still awaiting a result (covers models that emit empty tool-call ids).
        let matched = false;
        const items = r.items.map((it) => {
          if (matched || it.kind !== "tool" || it.result !== undefined) return it;
          if (ev.call_id && it.callId === ev.call_id) {
            matched = true;
            return { ...it, result: ev.result, ok: ev.ok };
          }
          return it;
        });
        if (!matched) {
          for (let i = items.length - 1; i >= 0; i--) {
            if (items[i].kind === "tool" && items[i].result === undefined) {
              items[i] = { ...items[i], result: ev.result, ok: ev.ok };
              matched = true;
              break;
            }
          }
        }
        return { ...r, items };
      });
    case "subagent_done":
      return upsertRun(state, ev.run_id, (r) => ({
        ...r,
        state: ev.state ?? r.state,
        summary: ev.summary ?? r.summary,
        endedAt: ev.ended_at ?? r.endedAt,
      }));

    // ── Memory ──
    case "memory_list":
      return { ...state, memories: Array.isArray(ev.memories) ? ev.memories : [] };
    case "memory_saved": {
      const ok = !ev.deleted;
      return {
        ...state,
        toasts: pushToast(state.toasts, ok ? "success" : "info", ev.message ?? (ok ? "Memory saved" : "Memory forgotten")),
      };
    }

    // ── Plugins ──
    case "plugins_list":
      return { ...state, plugins: Array.isArray(ev.plugins) ? ev.plugins : [] };
    case "plugin_installed":
    case "plugin_removed":
    case "plugin_enabled":
    case "plugin_disabled": {
      const verb = ev.type.replace("plugin_", "");
      const msg = ("message" in ev && ev.message) || `${verb}: ${ev.name}`;
      return {
        ...state,
        toasts: pushToast(state.toasts, ev.ok === false ? "error" : "success", msg),
      };
    }
    case "plugin_error":
      return { ...state, toasts: pushToast(state.toasts, "error", ev.message) };

    // ── Skills ──
    case "skills":
      return { ...state, skills: Array.isArray(ev.skills) ? ev.skills : [] };

    // ── Vision ──
    case "vision_config":
      return { ...state, visionConfig: { vision_models: ev.vision_models ?? [], vision_model: ev.vision_model ?? null } };

    // ── Projects / workspace ──
    case "projects":
      return { ...state, projects: Array.isArray(ev.projects) ? ev.projects : [] };
    case "workspace_changed":
      return {
        ...state,
        workspace: ev.workspace,
        projects: Array.isArray(ev.projects) ? ev.projects : state.projects,
        switching: false,
        messages: [],
        currentAssistantId: null,
        streaming: false,
        pendingApproval: null,
        pendingAsk: null,
        sessions: [],
        currentSessionFile: null,
        stats: null,
        workState: null,
      };
    case "session_renamed": {
      const sessions = state.sessions.map((s) =>
        s.name === ev.name ? { ...s, title: ev.title || undefined } : s,
      );
      return { ...state, sessions };
    }

    // ── Compaction / config ──
    case "digested": {
      const n = ev.results;
      return {
        ...state,
        toasts: pushToast(
          state.toasts,
          "info",
          n > 1
            ? `Compacted ${n} large result(s) — ${ev.before_tokens.toLocaleString()} → ${ev.after_tokens.toLocaleString()} tokens`
            : `Compacted a large result — ${ev.before_tokens.toLocaleString()} → ${ev.after_tokens.toLocaleString()} tokens`,
        ),
      };
    }
    case "config_changed":
      return { ...state, toasts: pushToast(state.toasts, "info", `${ev.key} → ${ev.value}`) };

    // ── OAuth / lifecycle status ──
    case "oauth_prompt":
      // The core needs the user to visit an authorize URL (and, for the device
      // flow, paste back a code). Surface it as a blocking banner + a toast.
      return {
        ...state,
        pendingOauth: { url: ev.url, code: ev.code, message: ev.message },
        toasts: pushToast(state.toasts, "info", ev.message || "Complete the OAuth login in your browser"),
      };
    case "reflecting": {
      // Auto-reflect injected a reflection continuation turn. Mirror the TUI's
      // logInfo so the post-finish model activity isn't a mystery.
      const n = String(ev.recurrence ?? "");
      return {
        ...state,
        toasts: pushToast(
          state.toasts,
          "info",
          n && n !== "0"
            ? `auto-reflect: reflecting on this turn (${n} recurring patterns)…`
            : "auto-reflect: reflecting on this turn…",
        ),
      };
    }
    case "work_state":
      // Rolling KV-cache-aware work-state summary (goal/done/doing/next/files).
      return {
        ...state,
        workState: {
          version: ev.version,
          goal: ev.goal,
          done: ev.done,
          in_progress: ev.in_progress,
          next: ev.next,
          recent_files: ev.recent_files,
          last_activity: ev.last_activity,
        },
      };

    default:
      return state;
  }
}

// Re-exported for the bridge to narrow CoreEvent typing.
export type { CoreEvent };
