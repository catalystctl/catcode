"use client";

// useAgent — the client hook that owns AgentState.
//
// Opens an EventSource to /api/stream, hydrates from the server snapshot, then
// reduces every live core event. Exposes typed actions (prompt, steer, abort,
// approve, setKey, …) that POST a raw core command to /api/command and apply the
// optimistic `_user` event locally (the bridge tracks it for snapshots).

import { useCallback, useEffect, useRef, useState } from "react";
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
  setModel: (id: string) => void;
  setThinking: (level: string) => void;
  setApproval: (mode: "never" | "destructive" | "always") => Promise<void>;
  newSession: () => Promise<void>;
  loadSession: (path: string) => Promise<void>;
  listSessions: () => Promise<void>;
  compact: () => Promise<void>;
  reset: () => Promise<void>;
  stats: () => Promise<void>;
  dismissToast: (id: string) => void;
  // ── Subagent / intercom ──
  intercomReply: (reply: string) => Promise<void>;
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
  // ── Vision ──
  getVisionConfig: () => Promise<void>;
  setVisionConfig: (vision_model: string | null, vision_models?: string[]) => Promise<void>;
  // ── Config ──
  setConfig: (key: string, value: string | number) => Promise<void>;
}

export function useAgent(): AgentApi {
  const [state, setState] = useState<AgentState>(initialState);
  const [connected, setConnected] = useState(false);
  const stateRef = useRef(state);
  useEffect(() => {
    stateRef.current = state;
  }, [state]);

  // Stream connection + reducer.
  useEffect(() => {
    const es = new EventSource("/api/stream");
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
        setState((ev as { state: AgentState }).state);
      } else {
        setState((s) => reduce(s, ev as CoreEvent));
      }
    };
    return () => es.close();
  }, []);

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

  const post = useCallback(async (cmd: CoreCommand) => {
    try {
      await fetch("/api/command", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(cmd),
      });
    } catch {
      /* surfaced via the stream's error events */
    }
  }, []);

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

  const effortFor = useCallback((level: string): string | undefined => {
    return level === "off" || level === "minimal" ? undefined : level;
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

  const newSession = useCallback(() => send({ type: "new_session" }), [send]);
  const loadSession = useCallback((path: string) => send({ type: "load_session", path }), [send]);
  const listSessions = useCallback(() => send({ type: "list_sessions" }), [send]);
  const compact = useCallback(() => send({ type: "compact" }), [send]);
  const reset = useCallback(() => send({ type: "reset" }), [send]);
  const stats = useCallback(() => send({ type: "stats" }), [send]);

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

  return {
    state,
    connected,
    send,
    prompt,
    steer,
    abort,
    approve,
    setKey,
    setModel,
    setThinking,
    setApproval,
    newSession,
    loadSession,
    listSessions,
    compact,
    reset,
    stats,
    dismissToast,
    intercomReply,
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
    getVisionConfig,
    setVisionConfig,
    setConfig,
  };
}
