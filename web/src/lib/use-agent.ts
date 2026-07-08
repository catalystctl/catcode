"use client";

// useAgent — the client hook that owns AgentState.
//
// Opens an EventSource to /api/stream, hydrates from the server snapshot, then
// reduces every live core event. Exposes typed actions (prompt, steer, abort,
// approve, setKey, …) that POST a raw core command to /api/command and apply the
// optimistic `_user` event locally (the bridge tracks it for snapshots).

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { reduce, initialState } from "./reducer";
import type { AgentState, CoreCommand, CoreEvent, ModelInfo } from "./types";

// ponytail: localStorage persistence for UI preferences (model/thinking/approval).
// try/catch guards SSR + disabled-storage; failing silently is fine.
function lsGet(k: string): string | null {
  try {
    return typeof localStorage !== "undefined" ? localStorage.getItem(k) : null;
  } catch {
    return null;
  }
}
function lsSet(k: string, v: string): void {
  try {
    if (typeof localStorage !== "undefined") localStorage.setItem(k, v);
  } catch {
    /* ignore */
  }
}

function pickPreferredModel(models: ModelInfo[]): string | null {
  if (models.length === 0) return null;
  const preferred =
    models.find((m) => /glm[-_. ]?5/i.test(m.id)) ??
    models.find((m) => /5\.2|glm/i.test(m.id)) ??
    models[0];
  return preferred.id;
}

export interface AgentApi {
  state: AgentState;
  connected: boolean;
  /** Forward any core command (optimistic for send/steer). */
  send: (cmd: CoreCommand) => Promise<void>;
  prompt: (text: string, images?: string[]) => Promise<void>;
  steer: (text: string) => Promise<void>;
  abort: () => Promise<void>;
  approve: (decision: "yes" | "no" | "always") => Promise<void>;
  setKey: (key: string) => Promise<void>;
  setProvider: (name: string) => Promise<void>;
  login: (preset: string, key?: string) => Promise<void>;
  logout: (provider: string) => Promise<void>;
  listProviderPresets: () => Promise<void>;
  setModel: (id: string) => void;
  setThinking: (level: string) => void;
  setApproval: (mode: "never" | "destructive" | "always") => Promise<void>;
  newSession: () => Promise<void>;
  loadSession: (path: string) => Promise<void>;
  listSessions: () => Promise<void>;
  compact: (instructions?: string) => Promise<void>;
  reset: () => Promise<void>;
  stats: () => Promise<void>;
  context: () => Promise<void>;
  dismissToast: (id: string) => void;
  // ── Subagent / intercom ──
  intercomReply: (reply: string) => Promise<void>;
  // ── Ask tool ──
  askReply: (answers: Record<string, string> | null) => Promise<void>;
  // ── OAuth ──
  submitOauthCode: (code: string) => Promise<void>;
  dismissOauth: () => void;
  // ── Turn / history ──
  undo: () => Promise<void>;
  clear: () => Promise<void>;
  // ── Memory ──
  saveMemory: (text: string, tags?: string[]) => Promise<void>;
  listMemory: () => Promise<void>;
  forgetMemory: (id: string) => Promise<void>;
  // ── Plugins ──
  installPlugin: (path: string) => Promise<void>;
  removePlugin: (name: string) => Promise<void>;
  enablePlugin: (name: string) => Promise<void>;
  disablePlugin: (name: string) => Promise<void>;
  listPlugins: () => Promise<void>;
  // ── Skills ──
  listSkills: () => Promise<void>;
  applySkill: (name: string, task?: string) => Promise<void>;
  // ── Vision ──
  getVisionConfig: () => Promise<void>;
  setVisionConfig: (vision_model: string | null, vision_models?: string[]) => Promise<void>;
  // ── Config ──
  setConfig: (key: string, value: string | number) => Promise<void>;
  // ── Projects / workspace ──
  switchWorkspace: (path: string) => Promise<void>;
  renameSession: (name: string, title: string) => Promise<void>;
  listProjects: () => Promise<void>;
  addProject: (path: string) => Promise<void>;
  removeProject: (path: string) => Promise<void>;
  // ── Session lifecycle ──
  deleteSession: (path: string) => Promise<void>;
  // ── Connection ──
  reconnect: () => void;
  // ── Utility ──
  copyLastReply: () => void;
  exportTranscript: () => string;
}

export function useAgent(): AgentApi {
  const [state, setState] = useState<AgentState>(initialState);
  const [connected, setConnected] = useState(false);
  const [reconnectKey, setReconnectKey] = useState(0);
  const stateRef = useRef(state);
  useEffect(() => {
    stateRef.current = state;
  }, [state]);

  // Multi-session: the client views ONE session at a time but many can be live
  // server-side. `streamSessionId` is the session the SSE stream is connected to
  // (null = the default workspace session); changing it reopens the stream.
  // `activeRef`/`workspaceRef` route commands to the viewed session and are kept
  // in sync from snapshots WITHOUT reopening the stream.
  const [streamSessionId, setStreamSessionId] = useState<string | null>(null);
  const activeRef = useRef<string | null>(null);
  const workspaceRef = useRef<string>("");
  useEffect(() => {
    if (state.workspace) workspaceRef.current = state.workspace;
    if (state.currentSessionFile) activeRef.current = state.currentSessionFile;
  }, [state.workspace, state.currentSessionFile]);

  // Stream connection + reducer. Re-runs when the viewed session changes
  // (streamSessionId) or on manual reconnect. EventSource auto-reconnects on
  // transient drops; reconnectKey gives the user an explicit force-fresh option.
  useEffect(() => {
    let url = "/api/stream";
    if (streamSessionId) {
      const params = new URLSearchParams();
      params.set("session", streamSessionId);
      if (workspaceRef.current) params.set("workspace", workspaceRef.current);
      url = `/api/stream?${params.toString()}`;
    }
    const es = new EventSource(url);
    es.onopen = () => setConnected(true);
    es.onerror = () => setConnected(false);
    es.onmessage = (e) => {
      let ev: CoreEvent | { type: "_snapshot"; state: AgentState };
      try {
        ev = JSON.parse(e.data);
      } catch {
        return;
      }
      if (ev.type === "_snapshot") {
        const snap = (ev as { state: AgentState }).state;
        // Preserve the user's UI prefs (model/thinking) across session switches:
        // each session's core reports its own (default) values, but the choice
        // is global (localStorage).
        const t = lsGet("umans:thinking");
        const m = lsGet("umans:model");
        setState({
          ...snap,
          thinkingLevel: t ?? snap.thinkingLevel,
          selectedModel:
            m && snap.models.some((x) => x.id === m) ? m : snap.selectedModel,
        });
      } else {
        setState((s) => reduce(s, ev as CoreEvent));
      }
    };
    return () => es.close();
  }, [streamSessionId, reconnectKey]);

  // Auto-select a model once they arrive and none is chosen. Prefer a saved
  // selection from localStorage so the user's last model persists across reloads.
  useEffect(() => {
    if (stateRef.current.selectedModel || stateRef.current.models.length === 0) return;
    const saved = lsGet("umans:model");
    const id =
      saved && stateRef.current.models.some((m) => m.id === saved)
        ? saved
        : pickPreferredModel(stateRef.current.models);
    if (id) setState((s) => reduce(s, { type: "_select_model", id }));
  }, [state.models]);

  // Restore saved UI preferences (thinking level, approval mode) on first mount.
  useEffect(() => {
    const t = lsGet("umans:thinking");
    if (t) setState((s) => reduce(s, { type: "_set_thinking", level: t }));
    const a = lsGet("umans:approval");
    if (a === "never" || a === "destructive" || a === "always") {
      fetch("/api/command", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ type: "set_approval", mode: a }),
      }).catch(() => {
        /* surfaced via the stream */
      });
    }
  }, []);

  const post = useCallback(
    async (
      cmd: CoreCommand,
    ): Promise<{ ok?: boolean; session?: string; workspace?: string; error?: string }> => {
      // Attach routing metadata so the bridge targets the viewed session.
      const body: Record<string, unknown> = { ...cmd };
      if (activeRef.current) body.session = activeRef.current;
      if (workspaceRef.current) body.workspace = workspaceRef.current;
      try {
        const res = await fetch("/api/command", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
        });
        return (await res.json().catch(() => ({}))) as {
          ok?: boolean;
          session?: string;
          workspace?: string;
          error?: string;
        };
      } catch {
        /* surfaced via the stream's error events */
        return {};
      }
    },
    [],
  );

  // Switch the viewed session: reopen the SSE stream for it (the bridge starts
  // its core if it isn't already live) and re-apply the user's global approval
  // preference to the new session's core. Other live sessions keep running.
  const switchToSession = useCallback(
    (sessionFile: string, workspace?: string) => {
      activeRef.current = sessionFile;
      if (workspace) workspaceRef.current = workspace;
      // Optimistically blank the view while the new session's snapshot loads.
      setState((s) => ({
        ...s,
        messages: [],
        currentAssistantId: null,
        streaming: false,
        retrying: false,
        pendingApproval: null,
        pendingIntercom: null,
        pendingAsk: null,
        currentSessionFile: sessionFile,
        switching: true,
        workspace: workspace ?? s.workspace,
      }));
      setStreamSessionId(sessionFile);
      // Re-apply the user's approval preference to this session's core.
      const a = lsGet("umans:approval");
      if (a === "never" || a === "destructive" || a === "always") {
        void post({ type: "set_approval", mode: a });
      }
    },
    [post],
  );

  const send = useCallback(
    async (cmd: CoreCommand) => {
      if (cmd.type === "send" || cmd.type === "steer") {
        setState((s) =>
          reduce(s, {
            type: "_user",
            text: cmd.prompt,
            model: cmd.model,
            steer: cmd.type === "steer",
          }),
        );
      }
      await post(cmd);
    },
    [post],
  );

  // Refresh the discoverable-skills list when a turn ends (streaming → idle)
  // so a skill created mid-session (e.g. by /reflect or /index) shows up in the
  // /skill:<name> autocomplete without a reconnect. Skips the initial false→false.
  const wasStreamingRef = useRef(false);
  useEffect(() => {
    if (wasStreamingRef.current && !state.streaming) {
      post({ type: "list_skills" }).catch(() => {});
    }
    wasStreamingRef.current = state.streaming;
  }, [state.streaming, post]);

  const effortFor = useCallback((level: string): string | undefined => {
    return level === "off" ? undefined : level;
  }, []);

  const prompt = useCallback(
    async (text: string, images?: string[]) => {
      const s = stateRef.current;
      const model = s.selectedModel ?? s.models[0]?.id ?? "";
      const cmd: CoreCommand = { type: "send", prompt: text, model };
      const eff = effortFor(s.thinkingLevel);
      if (eff) cmd.reasoning_effort = eff;
      if (images?.length) cmd.images = images;
      await send(cmd);
    },
    [send, effortFor],
  );

  const steer = useCallback(
    async (text: string) => {
      const s = stateRef.current;
      const model = s.selectedModel ?? s.models[0]?.id ?? "";
      const cmd: CoreCommand = { type: "steer", prompt: text, model };
      const eff = effortFor(s.thinkingLevel);
      if (eff) cmd.reasoning_effort = eff;
      await send(cmd);
    },
    [send, effortFor],
  );

  const abort = useCallback(() => send({ type: "abort" }), [send]);

  const approve = useCallback(
    (decision: "yes" | "no" | "always") => {
      const s = stateRef.current;
      const req = s.pendingApproval;
      if (!req) return Promise.resolve();
      // Optimistically clear the banner so double-clicks don't re-fire.
      setState((st) => ({ ...st, pendingApproval: null }));
      return send({ type: "approve", request_id: req.request_id, decision });
    },
    [send],
  );

  const setKey = useCallback(
    async (key: string) => {
      const s = stateRef.current;
      await send({ type: "set_key", api_key: key, provider: s.provider || undefined });
      // Re-discover models now that a key is set (the core doesn't on set_key).
      if (s.models.length === 0 && s.provider) {
        await post({ type: "set_provider", name: s.provider });
      }
    },
    [send, post],
  );

  // Log in to a first-party provider (OpenAI/Codex, Gemini, Anthropic). When
  // no key is passed the core reads it from the preset's standard env var.
  // Multiple providers can be logged in at once; the core re-aggregates models
  // so the provider's models appear in /models alongside any others.
  const login = useCallback(
    async (preset: string, key?: string) => {
      await send({ type: "login", preset, api_key: key });
    },
    [send],
  );

  // Switch the default/fallback provider (a turn still routes per-model, but
  // compaction/legacy fallback uses this one).
  const setProvider = useCallback(async (name: string) => {
    await send({ type: "set_provider", name });
  }, [send]);

  // Log out of a provider: the core drops its key/config and re-aggregates.
  const logout = useCallback(async (provider: string) => {
    await send({ type: "logout", provider });
  }, [send]);

  // Ask the core for the latest preset list (configured/hasKey/loggedIn flags).
  const listProviderPresets = useCallback(async () => {
    await post({ type: "list_provider_presets" });
  }, [post]);

  const setModel = useCallback((id: string) => {
    lsSet("umans:model", id);
    setState((s) => reduce(s, { type: "_select_model", id }));
  }, []);

  const setThinking = useCallback((level: string) => {
    lsSet("umans:thinking", level);
    setState((s) => reduce(s, { type: "_set_thinking", level }));
  }, []);

  const setApproval = useCallback(
    (mode: "never" | "destructive" | "always") => {
      lsSet("umans:approval", mode);
      return send({ type: "set_approval", mode });
    },
    [send],
  );

  const newSession = useCallback(async () => {
    const r = await post({ type: "new_session" });
    if (r.session) switchToSession(r.session, r.workspace);
  }, [post, switchToSession]);
  const loadSession = useCallback(
    (path: string): Promise<void> => {
      switchToSession(path);
      return Promise.resolve();
    },
    [switchToSession],
  );
  const listSessions = useCallback(() => send({ type: "list_sessions" }), [send]);
  const compact = useCallback(
    (instructions?: string) =>
      send({ type: "compact", ...(instructions ? { instructions } : {}) }),
    [send],
  );
  const reset = useCallback(() => send({ type: "reset" }), [send]);
  const stats = useCallback(() => send({ type: "stats" }), [send]);
  const context = useCallback(() => send({ type: "context" }), [send]);

  const dismissToast = useCallback((id: string) => {
    setState((s) => reduce(s, { type: "_dismiss_toast", id }));
  }, []);

  // ── Subagent / intercom ──
  const intercomReply = useCallback(
    (reply: string) => {
      const s = stateRef.current;
      const req = s.pendingIntercom;
      if (!req) return Promise.resolve();
      setState((st) => ({ ...st, pendingIntercom: null }));
      return send({ type: "intercom_reply", request_id: req.request_id, reply });
    },
    [send],
  );

  // ── Ask tool ──
  // Submit the user's answers to a pending `ask` tool call (object keyed by
  // question id), or pass null to skip the prompt.
  const askReply = useCallback(
    (answers: Record<string, string> | null) => {
      const s = stateRef.current;
      const req = s.pendingAsk;
      if (!req) return Promise.resolve();
      setState((st) => ({ ...st, pendingAsk: null }));
      return send({ type: "ask_reply", request_id: req.request_id, answers });
    },
    [send],
  );

  // ── OAuth ──
  // Complete a no-browser (manual-code) OAuth login by pasting the code or
  // final callback URL the provider returned. Mirrors the TUI's /oauth-code.
  const submitOauthCode = useCallback(
    async (code: string) => {
      const v = code.trim();
      if (!v) return;
      await post({ type: "oauth_code", code: v });
    },
    [post],
  );
  const dismissOauth = useCallback(() => {
    // Hide the banner only — the core keeps its pending login until oauth_code
    // or a fresh /login, so dismissing does not cancel an in-flight login.
    setState((s) => ({ ...s, pendingOauth: null }));
  }, []);

  // ── Turn / history ──
  const undo = useCallback(() => send({ type: "undo" }), [send]);
  const clear = useCallback(() => send({ type: "clear" }), [send]);

  // ── Memory ──
  const saveMemory = useCallback(
    async (text: string, tags?: string[]) => {
      await send({ type: "save_memory", text, tags });
      await post({ type: "list_memory" });
    },
    [send, post],
  );
  const listMemory = useCallback(() => send({ type: "list_memory" }), [send]);
  const forgetMemory = useCallback(
    async (id: string) => {
      await send({ type: "forget_memory", id });
      await post({ type: "list_memory" });
    },
    [send, post],
  );

  // ── Plugins ──
  const installPlugin = useCallback(
    async (path: string) => {
      await send({ type: "install_plugin", path });
      await post({ type: "list_plugins" });
    },
    [send, post],
  );
  const removePlugin = useCallback(
    async (name: string) => {
      await send({ type: "remove_plugin", name });
      await post({ type: "list_plugins" });
    },
    [send, post],
  );
  const enablePlugin = useCallback(
    async (name: string) => {
      await send({ type: "enable_plugin", name });
      await post({ type: "list_plugins" });
    },
    [send, post],
  );
  const disablePlugin = useCallback(
    async (name: string) => {
      await send({ type: "disable_plugin", name });
      await post({ type: "list_plugins" });
    },
    [send, post],
  );
  const listPlugins = useCallback(() => send({ type: "list_plugins" }), [send]);

  // ── Skills ──
  const listSkills = useCallback(() => send({ type: "list_skills" }), [send]);
  const applySkill = useCallback(
    async (name: string, task?: string) => {
      const s = stateRef.current;
      const model = s.selectedModel ?? s.models[0]?.id ?? "";
      const cmd: CoreCommand = { type: "apply_skill", name, model };
      const eff = effortFor(s.thinkingLevel);
      if (eff) cmd.reasoning_effort = eff;
      if (task && task.trim()) cmd.task = task.trim();
      // Optimistic user line: show the concise /skill:<name> [task] the user
      // invoked (the core inlines the full skill body into the actual prompt).
      const display = task && task.trim() ? `/skill:${name} ${task.trim()}` : `/skill:${name}`;
      setState((st) => reduce(st, { type: "_user", text: display, model, steer: false }));
      await post(cmd);
    },
    [post, effortFor],
  );

  // ── Vision ──
  const getVisionConfig = useCallback(() => send({ type: "get_vision_config" }), [send]);
  const setVisionConfig = useCallback(
    (vision_model: string | null, vision_models?: string[]) =>
      send({ type: "set_vision_config", vision_model, vision_models }),
    [send],
  );

  // ── Config ──
  const setConfig = useCallback(
    (key: string, value: string | number) => send({ type: "set_config", key, value }),
    [send],
  );

  // ── Projects / workspace ──
  const switchWorkspace = useCallback(
    async (path: string) => {
      const r = await post({ type: "switch_workspace", path });
      if (r.session) switchToSession(r.session, r.workspace ?? path);
    },
    [post, switchToSession],
  );
  const renameSession = useCallback(
    (name: string, title: string) => send({ type: "rename_session", name, title }),
    [send],
  );
  const listProjects = useCallback(() => send({ type: "list_projects" }), [send]);
  const addProject = useCallback((path: string) => send({ type: "add_project", path }), [send]);
  const removeProject = useCallback(
    (path: string) => send({ type: "remove_project", path }),
    [send],
  );

  // ── Session lifecycle ──
  const deleteSession = useCallback(
    async (path: string) => {
      const r = await post({ type: "delete_session", path });
      // If we deleted the session currently being viewed, switch to the
      // next most-recent one (returned by the bridge) so the view stays live.
      if (activeRef.current && activeRef.current === path && r.session) {
        switchToSession(r.session, r.workspace);
      }
    },
    [post, switchToSession],
  );

  // ── Connection ──
  const reconnect = useCallback(() => setReconnectKey((k) => k + 1), []);

  // ── Utility ──
  const copyLastReply = useCallback(() => {
    const msgs = stateRef.current.messages;
    for (let i = msgs.length - 1; i >= 0; i--) {
      const m = msgs[i];
      if (m.role === "assistant" && m.text.trim()) {
        navigator.clipboard?.writeText(m.text).then(
          () => {},
          () => {},
        );
        return;
      }
    }
  }, []);

  const exportTranscript = useCallback((): string => {
    const msgs = stateRef.current.messages;
    const lines: string[] = [`# Catalyst Code Transcript`, ``];
    for (const m of msgs) {
      if (m.role === "user") {
        lines.push(`## You`, ``, m.text, ``);
      } else if (m.role === "assistant") {
        lines.push(`## Assistant${m.model ? ` (${m.model})` : ""}`, ``);
        if (m.text) lines.push(m.text, ``);
        for (const tc of m.toolCalls) {
          lines.push(
            `> **Tool: ${tc.name}**`,
            `> \`\`\``,
            `> ${(tc.argString || "").split("\n").join("\n> ")}`,
            `> \`\`\``,
            tc.result ? `> _${tc.result.ok ? "ok" : "error"}_` : `> _running_`,
            ``,
          );
          if (tc.result?.output) {
            lines.push(`> \`\`\``, `> ${tc.result.output.split("\n").join("\n> ")}`, `> \`\`\``, ``);
          }
        }
      }
    }
    return lines.join("\n");
  }, []);

  return useMemo(
    () => ({
      state,
      connected,
      send,
      prompt,
      steer,
      abort,
      approve,
      setKey,
      setProvider,
      login,
      logout,
      listProviderPresets,
      setModel,
      setThinking,
      setApproval,
      newSession,
      loadSession,
      listSessions,
      compact,
      reset,
      stats,
      context,
      dismissToast,
      intercomReply,
      askReply,
      submitOauthCode,
      dismissOauth,
      undo,
      clear,
      saveMemory,
      listMemory,
      forgetMemory,
      installPlugin,
      removePlugin,
      enablePlugin,
      disablePlugin,
      listPlugins,
      listSkills,
      applySkill,
      getVisionConfig,
      setVisionConfig,
      setConfig,
      switchWorkspace,
      renameSession,
      listProjects,
      addProject,
      removeProject,
      deleteSession,
      reconnect,
      copyLastReply,
      exportTranscript,
    }),
    // Every action above is a useCallback with stable deps (refs/empty), so the
    // returned object identity only changes when `state` or `connected` do.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [state, connected],
  );
}
