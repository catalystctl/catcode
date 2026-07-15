"use client";

// Client-only IDE layout/panel state. This NEVER touches the core reducer,
// AgentState, or the SSE snapshot — it is a completely separate state slice so
// the web-event-fieldname-mismatch + snapshot round-trip contracts stay intact.
// Layout prefs and terminal session metadata persist to localStorage (scoped
// per project/workspace). File tabs, gitStatus, and preview stay in-memory.
// Server-side PTYs outlive the browser tab; the client only stores ids so a
// hard refresh can reattach. The hook does no I/O: components own their own
// fetch/WS lifecycle and call back into the hook to set state.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  IdeLayoutState,
  IdePanelId,
  IdeTab,
  TerminalSession,
  GitStatus,
  PreviewState,
  DockPosition,
  MovablePanelId,
} from "./types";
import { basename, detectLanguage } from "./lang";
import { disposeEditorModel } from "./editor-model-registry";

const STORAGE_KEY = "catcode:ide-layout";

function storageKey(workspace?: string): string {
  return workspace ? `${STORAGE_KEY}:${encodeURIComponent(workspace)}` : STORAGE_KEY;
}

/** Keys persisted across reloads (the rest is in-memory only). */
const PERSISTED: (keyof IdeLayoutState)[] = [
  "activePanel",
  "sidebarWidth",
  "sidebarCollapsed",
  "bottomPanelHeight",
  "bottomPanelVisible",
  "copilotVisible",
  "copilotWidth",
  "panelLocations",
  "panelVisibility",
  "activeDockPanels",
  "leftDockWidth",
  "expandedDirs",
  // Terminal tabs are per-project and reattach to server PTYs after refresh.
  "terminals",
  "activeTerminalId",
];

const SESSION_ID_RE = /^[A-Za-z0-9_-]{1,128}$/;

function sanitizeTerminals(value: unknown): TerminalSession[] {
  if (!Array.isArray(value)) return [];
  const out: TerminalSession[] = [];
  for (const item of value) {
    if (!item || typeof item !== "object") continue;
    const raw = item as Partial<TerminalSession>;
    if (typeof raw.id !== "string" || !SESSION_ID_RE.test(raw.id)) continue;
    // Dead shells are not restored — a refresh after exit should not spawn a
    // replacement PTY under the old tab. Live ones reattach to the server.
    if (raw.alive === false || typeof raw.exitCode === "number") continue;
    out.push({
      id: raw.id,
      title: typeof raw.title === "string" && raw.title.trim() ? raw.title : `Terminal ${out.length + 1}`,
      cwd: typeof raw.cwd === "string" ? raw.cwd : "",
      alive: true,
      exitCode: null,
    });
  }
  return out;
}

function sanitizeActiveTerminalId(value: unknown, terminals: TerminalSession[]): string | null {
  if (typeof value !== "string" || !SESSION_ID_RE.test(value)) return null;
  return terminals.some((t) => t.id === value) ? value : (terminals[terminals.length - 1]?.id ?? null);
}

const DEFAULTS: IdeLayoutState = {
  activePanel: "explorer",
  openTabs: [],
  activeTabId: null,
  sidebarWidth: 256,
  sidebarCollapsed: false,
  bottomPanelHeight: 220,
  bottomPanelVisible: false,
  copilotVisible: true,
  copilotWidth: 440,
  panelLocations: { chat: "right", terminal: "bottom", git: "left", preview: "main" },
  panelVisibility: { chat: true, terminal: false, git: false, preview: false },
  activeDockPanels: { left: null, right: "chat", bottom: null, main: null },
  leftDockWidth: 360,
  terminals: [],
  activeTerminalId: null,
  gitStatus: null,
  preview: { kind: "none", target: "" },
  expandedDirs: [],
};

function loadPersisted(key: string): Partial<IdeLayoutState> {
  if (typeof window === "undefined") return {};
  try {
    // The unscoped value is a migration fallback for existing installations.
    const raw = window.localStorage.getItem(key) ??
      (key !== STORAGE_KEY ? window.localStorage.getItem(STORAGE_KEY) : null);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return {};
    const out: Partial<IdeLayoutState> = {};
    for (const k of PERSISTED) {
      if (k === "terminals" || k === "activeTerminalId") continue;
      if (k in parsed) (out as Record<string, unknown>)[k] = parsed[k];
    }
    const terminals = sanitizeTerminals((parsed as IdeLayoutState).terminals);
    out.terminals = terminals;
    out.activeTerminalId = sanitizeActiveTerminalId(
      (parsed as IdeLayoutState).activeTerminalId,
      terminals,
    );
    return out;
  } catch {
    return {};
  }
}



export interface IdeApi {
  state: IdeLayoutState;
  // ── panels / layout ──
  setActivePanel: (p: IdePanelId) => void;
  /** VSCode activity-bar behavior: clicking the active icon collapses the sidebar;
   *  clicking another switches + expands. */
  togglePanel: (p: IdePanelId) => void;
  toggleSidebar: () => void;
  setSidebarWidth: (px: number) => void;
  setBottomPanelHeight: (px: number) => void;
  toggleBottomPanel: () => void;
  toggleCopilot: () => void;
  setCopilotWidth: (px: number) => void;
  setLeftDockWidth: (px: number) => void;
  movePanel: (panel: MovablePanelId, position: DockPosition) => void;
  toggleDockPanel: (panel: MovablePanelId) => void;
  hideDockPanel: (panel: MovablePanelId) => void;
  showDockPanel: (panel: MovablePanelId) => void;
  selectDockPanel: (position: DockPosition, panel: MovablePanelId) => void;
  selectExplorer: () => void;
  selectEditor: () => void;
  // ── tabs / files ──
  openFile: (path: string, language?: string) => void;
  closeTab: (id: string) => void;
  setActiveTab: (id: string) => void;
  markDirty: (id: string, dirty: boolean) => void;
  // ── terminal ──
  newTerminal: (cwd?: string) => string;
  /** Focus (or create) a terminal and queue a command for its shell stdin. The
   *  actual WS write is performed by the Terminal component (it owns the WS);
   *  the hook only manages session state + a pending-command queue. */
  runCommand: (command: string, cwd?: string) => void;
  closeTerminal: (id: string) => void;
  setActiveTerminal: (id: string) => void;
  setTerminalExit: (id: string, code: number) => void;
  // ── git ──
  setGitStatus: (s: GitStatus | null) => void;
  refreshGit: () => void;
  // ── preview ──
  setPreview: (p: PreviewState) => void;
  // ── tree ──
  toggleDir: (path: string) => void;
  isExpanded: (path: string) => boolean;
}

export function useIde(workspace?: string): IdeApi {
  // The server and the client's hydration pass must render the same layout.
  // Restore local preferences only after hydration; reading localStorage in the
  // useState initializer makes the client render saved values while the server
  // renders DEFAULTS, which causes React to discard the entire IDE shell.
  const [state, setState] = useState<IdeLayoutState>(DEFAULTS);
  const key = storageKey(workspace);
  const [restoredKey, setRestoredKey] = useState<string | null>(null);

  useEffect(() => {
    // Layout + terminal tabs are scoped per project key. File tabs, git, and
    // preview reset so they never silently point at the previous workspace.
    const loaded = loadPersisted(key);
    setState({ ...DEFAULTS, ...loaded });
    setRestoredKey(key);
  }, [key]);

  // Persist the persisted subset whenever it changes.
  useEffect(() => {
    // Do not write DEFAULTS before the restoration effect has had a chance to
    // read the user's saved layout.
    if (restoredKey !== key || typeof window === "undefined") return;
    try {
      const slice: Record<string, unknown> = {};
      for (const k of PERSISTED) {
        if (k === "terminals") {
          // Only durable live sessions — exited tabs are ephemeral UI state.
          slice.terminals = state.terminals.filter((t) => t.alive && t.exitCode == null);
          continue;
        }
        if (k === "activeTerminalId") {
          const live = (slice.terminals as TerminalSession[] | undefined) ??
            state.terminals.filter((t) => t.alive && t.exitCode == null);
          const active = state.activeTerminalId;
          slice.activeTerminalId =
            active && live.some((t) => t.id === active)
              ? active
              : (live[live.length - 1]?.id ?? null);
          continue;
        }
        slice[k] = state[k];
      }
      window.localStorage.setItem(key, JSON.stringify(slice));
    } catch {
      /* ignore quota / private-mode errors */
    }
  }, [key, restoredKey, state]);

  // Mirror state into a ref so runCommand can read current session state without
  // a stale closure (and without mutating refs inside the setState updater, which
  // React strict mode would double-invoke).
  const stateRef = useRef(state);
  useEffect(() => {
    stateRef.current = state;
  }, [state]);

  // Pending terminal commands waiting for a Terminal component to drain (one
  // queue per active session id). The Terminal component pops + WS-writes these.
  const pendingCommands = useRef<Record<string, string[]>>({});
  const termSeq = useRef(0);

  const setActivePanel = useCallback((p: IdePanelId) => {
    setState((s) => ({ ...s, activePanel: p, sidebarCollapsed: false }));
  }, []);

  const togglePanel = useCallback((p: IdePanelId) => {
    setState((s) => {
      if (s.activePanel === p) {
        return { ...s, sidebarCollapsed: !s.sidebarCollapsed };
      }
      return { ...s, activePanel: p, sidebarCollapsed: false };
    });
  }, []);

  const toggleSidebar = useCallback(() => {
    setState((s) => ({ ...s, sidebarCollapsed: !s.sidebarCollapsed }));
  }, []);

  const setSidebarWidth = useCallback((px: number) => {
    setState((s) => ({ ...s, sidebarWidth: clamp(px, 160, 640) }));
  }, []);

  const setBottomPanelHeight = useCallback((px: number) => {
    setState((s) => ({ ...s, bottomPanelHeight: clamp(px, 0, 800) }));
  }, []);

  const toggleBottomPanel = useCallback(() => {
    setState((s) => ({
      ...s,
      bottomPanelVisible: !s.bottomPanelVisible,
      panelVisibility: { ...s.panelVisibility, terminal: !s.bottomPanelVisible },
      activeDockPanels: { ...s.activeDockPanels, bottom: !s.bottomPanelVisible ? "terminal" : s.activeDockPanels.bottom },
    }));
  }, []);

  const toggleCopilot = useCallback(() => {
    setState((s) => {
      const position = s.panelLocations.chat;
      const active = s.panelVisibility.chat && s.activeDockPanels[position] === "chat";
      return {
        ...s,
        copilotVisible: !active,
        panelVisibility: { ...s.panelVisibility, chat: !active },
        activeDockPanels: !active
          ? { ...s.activeDockPanels, [position]: "chat" }
          : s.activeDockPanels,
      };
    });
  }, []);

  const setCopilotWidth = useCallback((px: number) => {
    setState((s) => ({ ...s, copilotWidth: clamp(px, 320, 900) }));
  }, []);

  const setLeftDockWidth = useCallback((px: number) => {
    setState((s) => ({ ...s, leftDockWidth: clamp(px, 260, 720) }));
  }, []);

  const movePanel = useCallback((panel: MovablePanelId, position: DockPosition) => {
    setState((s) => {
      const prev = s.panelLocations[panel];
      const activeDockPanels: IdeLayoutState["activeDockPanels"] = {
        ...s.activeDockPanels,
        [position]: panel,
      };
      // Clear the old dock's active pointer when this panel leaves it.
      if (prev !== position && activeDockPanels[prev] === panel) {
        const remaining =
          (["chat", "terminal", "git", "preview"] as const).find(
            (p) => p !== panel && s.panelVisibility[p] && s.panelLocations[p] === prev,
          ) ?? null;
        activeDockPanels[prev] = remaining;
      }
      const bottomPanelVisible = (["chat", "terminal", "git", "preview"] as const).some((p) => {
        const loc = p === panel ? position : s.panelLocations[p];
        const visible = p === panel ? true : s.panelVisibility[p];
        return visible && loc === "bottom";
      });
      return {
        ...s,
        panelLocations: { ...s.panelLocations, [panel]: position },
        panelVisibility: { ...s.panelVisibility, [panel]: true },
        copilotVisible: panel === "chat" ? true : s.copilotVisible,
        bottomPanelVisible,
        sidebarCollapsed: position === "left" ? false : s.sidebarCollapsed,
        activeDockPanels,
      };
    });
  }, []);

  const toggleDockPanel = useCallback((panel: MovablePanelId) => {
    setState((s) => {
      const position = s.panelLocations[panel];
      const active = s.panelVisibility[panel] && s.activeDockPanels[position] === panel;
      const visible = !active;
      return {
        ...s,
        panelVisibility: { ...s.panelVisibility, [panel]: visible },
        copilotVisible: panel === "chat" ? visible : s.copilotVisible,
        bottomPanelVisible: panel === "terminal" && position === "bottom" ? visible : s.bottomPanelVisible,
        sidebarCollapsed: visible && position === "left" ? false : s.sidebarCollapsed,
        activeDockPanels: visible
          ? { ...s.activeDockPanels, [position]: panel }
          : s.activeDockPanels,
      };
    });
  }, []);

  const showDockPanel = useCallback((panel: MovablePanelId) => {
    setState((s) => {
      const position = s.panelLocations[panel];
      return {
        ...s,
        panelVisibility: { ...s.panelVisibility, [panel]: true },
        copilotVisible: panel === "chat" ? true : s.copilotVisible,
        bottomPanelVisible: panel === "terminal" && position === "bottom" ? true : s.bottomPanelVisible,
        sidebarCollapsed: position === "left" ? false : s.sidebarCollapsed,
        activeDockPanels: { ...s.activeDockPanels, [position]: panel },
      };
    });
  }, []);

  const hideDockPanel = useCallback((panel: MovablePanelId) => {
    setState((s) => ({
      ...s,
      panelVisibility: { ...s.panelVisibility, [panel]: false },
      copilotVisible: panel === "chat" ? false : s.copilotVisible,
      bottomPanelVisible:
        panel === "terminal" && s.panelLocations.terminal === "bottom"
          ? false
          : s.bottomPanelVisible,
    }));
  }, []);

  const selectDockPanel = useCallback((position: DockPosition, panel: MovablePanelId) => {
    setState((s) => ({
      ...s,
      activeDockPanels: { ...s.activeDockPanels, [position]: panel },
    }));
  }, []);

  const selectExplorer = useCallback(() => {
    setState((s) => ({
      ...s,
      sidebarCollapsed: false,
      activeDockPanels: { ...s.activeDockPanels, left: null },
    }));
  }, []);

  const selectEditor = useCallback(() => {
    setState((s) => ({
      ...s,
      activeDockPanels: { ...s.activeDockPanels, main: null },
    }));
  }, []);

  const openFile = useCallback((path: string, language?: string) => {
    setState((s) => {
      const existing = s.openTabs.find((t) => t.kind === "file" && t.target === path);
      if (existing) {
        return {
          ...s,
          activeTabId: existing.id,
          activeDockPanels: { ...s.activeDockPanels, main: null },
        };
      }
      const tab: IdeTab = {
        id: path,
        kind: "file",
        target: path,
        label: basename(path),
        dirty: false,
        language: language ?? detectLanguage(path),
      };
      return {
        ...s,
        openTabs: [...s.openTabs, tab],
        activeTabId: tab.id,
        activeDockPanels: { ...s.activeDockPanels, main: null },
      };
    });
  }, []);

  const closeTab = useCallback((id: string) => {
    if (stateRef.current.openTabs.some((tab) => tab.id === id)) {
      // Let React detach the visible editor before disposing the backing model.
      // Tab switches do not come through here, so their undo history survives.
      queueMicrotask(() => disposeEditorModel(id));
    }
    setState((s) => {
      const idx = s.openTabs.findIndex((t) => t.id === id);
      if (idx < 0) return s;
      const openTabs = s.openTabs.filter((t) => t.id !== id);
      let activeTabId = s.activeTabId;
      if (activeTabId === id) {
        activeTabId = openTabs.length
          ? openTabs[Math.max(0, idx - 1)].id
          : null;
      }
      return { ...s, openTabs, activeTabId };
    });
  }, []);

  const setActiveTab = useCallback((id: string) => {
    setState((s) => ({
      ...s,
      activeTabId: id,
      activeDockPanels: { ...s.activeDockPanels, main: null },
    }));
  }, []);

  const markDirty = useCallback((id: string, dirty: boolean) => {
    setState((s) => ({
      ...s,
      openTabs: s.openTabs.map((t) => (t.id === id ? { ...t, dirty } : t)),
    }));
  }, []);

  const newTerminal = useCallback((cwd?: string) => {
    const id = `term_${Date.now()}_${++termSeq.current}`;
    setState((s) => {
      const session: TerminalSession = {
        id,
        title: `Terminal ${s.terminals.length + 1}`,
        cwd: cwd ?? "",
        alive: true,
        exitCode: null,
      };
      return {
        ...s,
        terminals: [...s.terminals, session],
        activeTerminalId: id,
        bottomPanelVisible: s.panelLocations.terminal === "bottom" ? true : s.bottomPanelVisible,
        panelVisibility: { ...s.panelVisibility, terminal: true },
        activeDockPanels: {
          ...s.activeDockPanels,
          [s.panelLocations.terminal]: "terminal",
        },
      };
    });
    return id;
  }, []);

  const runCommand = useCallback((command: string, cwd?: string) => {
    const s = stateRef.current;
    const activeAlive =
      !!s.activeTerminalId &&
      s.terminals.some((t) => t.id === s.activeTerminalId && t.alive);
    let id = s.activeTerminalId ?? "";
    let terminals = s.terminals;
    if (!activeAlive) {
      id = `term_${Date.now()}_${++termSeq.current}`;
      terminals = [
        ...terminals,
        {
          id,
          title: `Terminal ${terminals.length + 1}`,
          cwd: cwd ?? "",
          alive: true,
          exitCode: null,
        },
      ];
    }
    // Queue the command for the Terminal component owning `id` to drain + write.
    (pendingCommands.current[id] ??= []).push(command + "\n");
    setState((prev) => ({
      ...prev,
      terminals: terminals === prev.terminals ? prev.terminals : terminals,
      activeTerminalId: id,
      bottomPanelVisible: prev.panelLocations.terminal === "bottom" ? true : prev.bottomPanelVisible,
      panelVisibility: { ...prev.panelVisibility, terminal: true },
      activeDockPanels: {
        ...prev.activeDockPanels,
        [prev.panelLocations.terminal]: "terminal",
      },
    }));
  }, []);

  const closeTerminal = useCallback((id: string) => {
    setState((s) => {
      const terminals = s.terminals.filter((t) => t.id !== id);
      let activeTerminalId = s.activeTerminalId;
      if (activeTerminalId === id) {
        activeTerminalId = terminals.length
          ? terminals[terminals.length - 1].id
          : null;
      }
      delete pendingCommands.current[id];
      return { ...s, terminals, activeTerminalId };
    });
  }, []);

  const setActiveTerminal = useCallback((id: string) => {
    setState((s) => ({ ...s, activeTerminalId: id, bottomPanelVisible: true }));
  }, []);

  const setTerminalExit = useCallback((id: string, code: number) => {
    setState((s) => ({
      ...s,
      terminals: s.terminals.map((t) =>
        t.id === id ? { ...t, alive: false, exitCode: code } : t,
      ),
    }));
  }, []);

  const setGitStatus = useCallback((g: GitStatus | null) => {
    setState((s) => ({ ...s, gitStatus: g }));
  }, []);

  // Pure state hook: refreshGit is a no-op tick here; the caller (git-panel or
  // status-bar) performs the actual GET /api/git fetch then calls setGitStatus.
  const gitTick = useRef(0);
  const refreshGit = useCallback(() => {
    gitTick.current++;
  }, []);

  const setPreview = useCallback((p: PreviewState) => {
    setState((s) => ({ ...s, preview: p }));
  }, []);

  const toggleDir = useCallback((path: string) => {
    setState((s) => {
      const has = s.expandedDirs.includes(path);
      return {
        ...s,
        expandedDirs: has
          ? s.expandedDirs.filter((p) => p !== path)
          : [...s.expandedDirs, path],
      };
    });
  }, []);

  const isExpanded = useCallback(
    (path: string) => state.expandedDirs.includes(path),
    [state.expandedDirs],
  );

  // Memoize the returned api object so its identity is stable across renders
  // unless the state (or a state-derived callback) actually changed. The panels
  // (editor.tsx, git-panel.tsx) list `ide` in their useEffect deps; without this,
  // a parent re-render (e.g. IdeShell re-rendering on every streamed chat token)
  // would give them a fresh `ide` ref and re-fire their fetch effects — a fetch
  // storm during streaming. All callbacks below are stable (useCallback []);
  // only `state` and `isExpanded` (derived from state.expandedDirs) vary.
  return useMemo(
    () => ({
      state,
      setActivePanel,
      togglePanel,
      toggleSidebar,
      setSidebarWidth,
      setBottomPanelHeight,
      toggleBottomPanel,
      toggleCopilot,
      setCopilotWidth,
      setLeftDockWidth,
      movePanel,
      toggleDockPanel,
      hideDockPanel,
      showDockPanel,
      selectDockPanel,
      selectExplorer,
      selectEditor,
      openFile,
      closeTab,
      setActiveTab,
      markDirty,
      newTerminal,
      runCommand,
      closeTerminal,
      setActiveTerminal,
      setTerminalExit,
      setGitStatus,
      refreshGit,
      setPreview,
      toggleDir,
      isExpanded,
    }),
    [
      state,
      isExpanded,
      setActivePanel,
      togglePanel,
      toggleSidebar,
      setSidebarWidth,
      setBottomPanelHeight,
      toggleBottomPanel,
      toggleCopilot,
      setCopilotWidth,
      setLeftDockWidth,
      movePanel,
      toggleDockPanel,
      hideDockPanel,
      showDockPanel,
      selectDockPanel,
      selectExplorer,
      selectEditor,
      openFile,
      closeTab,
      setActiveTab,
      markDirty,
      newTerminal,
      runCommand,
      closeTerminal,
      setActiveTerminal,
      setTerminalExit,
      setGitStatus,
      refreshGit,
      setPreview,
      toggleDir,
    ],
  );
}

function clamp(n: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, Math.round(n)));
}
