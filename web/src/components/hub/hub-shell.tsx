"use client";

// The hub shell — a project-centric chat workspace:
//
//   • PROJECT TABS across the top. Add projects by browsing the machine
//     (ProjectSwitcher → /api/browse), or create/clone them.
//   • CHAT CENTER — full multi-session agent chat (SSE live feed to a pool of
//     catcode-core processes). Sessions keep running when you switch projects,
//     close the tab, or open another device — reconnecting rehydrates from the
//     server snapshot and resumes the live event stream.
//   • GIT SIDEBAR (collapsible, resizable) — full GitPanel bound to the active
//     project via an IdeContext shim.
//
// Account layout (GET/PUT /api/hub/layout) remembers open projects + the last
// viewed session per project so multi-device clients reattach the same chats.

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  BrandMark,
  FolderIcon,
  FolderPlusIcon,
  GitBranchIcon,
  PlusIcon,
  UserIcon,
  XIcon,
} from "@/components/icons";
import { ProjectSwitcher } from "@/components/ide/project-switcher";
import { ChatInner } from "@/components/chat";
import { SettingsModal } from "@/components/settings";
import { ErrorBoundary } from "@/components/error-boundary";
import { signOut } from "@/lib/auth-client";
import { useIsMobile } from "@/lib/use-media-query";
import { useBodyScrollLock } from "@/lib/use-body-scroll-lock";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { mergeRefs, useOutsideClose } from "@/lib/use-outside-close";
import {
  IdeContext,
  type AttachToChatFn,
  type IdeApi,
  type IdeContextValue,
} from "@/lib/ide-context";
import { useAgent } from "@/lib/use-agent";
import type { ProjectEntry } from "@/lib/types";
import { HubGitSidebar as GitSidebar } from "./git-sidebar";
import {
  defaultHubState,
  fetchHubLayout,
  pathBasename,
  pushHubLayout,
  sanitizeHubState,
  writeLocalHubCache,
  type HubPersistState,
} from "./hub-state";

/** Debounce for account-store writes (tab switches fire often). */
const HUB_SAVE_DEBOUNCE_MS = 400;
/** Poll so a second signed-in device picks up layout changes while open. */
const HUB_SYNC_POLL_MS = 4_000;

export function HubShell() {
  // ── state ─────────────────────────────────────────────────────────────────
  const [hub, setHub] = useState<HubPersistState>(defaultHubState);
  const [hydrated, setHydrated] = useState(false);
  const [projects, setProjects] = useState<ProjectEntry[]>([]);
  const [switcherOpen, setSwitcherOpen] = useState(false);
  const [switching, setSwitching] = useState(false);
  const [projectsError, setProjectsError] = useState<string | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [signingOut, setSigningOut] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  // Mobile: the git panel is a transient drawer (NOT persisted) so a small
  // screen never boots with chat covered; desktop keeps hub.gitOpen.
  const [mobileGitOpen, setMobileGitOpen] = useState(false);
  const gitResizeRef = useRef<{ startX: number; startWidth: number } | null>(null);
  const isMobile = useIsMobile();
  const menuRef = useOutsideClose(() => setMenuOpen(false), menuOpen);
  const menuTrapRef = useFocusTrap<HTMLDivElement>(menuOpen);
  const mobileGitCloseRef = useOutsideClose(
    () => setMobileGitOpen(false),
    isMobile && mobileGitOpen,
    { outsideClick: false },
  );
  const mobileGitTrapRef = useFocusTrap<HTMLElement>(isMobile && mobileGitOpen);
  useBodyScrollLock(isMobile && mobileGitOpen);

  const layoutUpdatedAtRef = useRef(0);
  const skipNextSaveRef = useRef(false);
  const hubRef = useRef(hub);
  hubRef.current = hub;
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // One agent hook for the whole shell — the bridge keeps every session's core
  // alive; switching projects just reopens the SSE stream on another LiveSession.
  const agent = useAgent();
  const attachRef = useRef<AttachToChatFn | null>(null);

  // Load the account layout (server is source of truth; localStorage is a
  // same-device cache + one-shot migration for pre-sync installs).
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const result = await fetchHubLayout();
        if (cancelled) return;
        layoutUpdatedAtRef.current = result.updatedAt;
        skipNextSaveRef.current = true;
        setHub(result.layout);
      } finally {
        if (!cancelled) setHydrated(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Debounced push to the account store on every local change.
  useEffect(() => {
    if (!hydrated) return;
    if (skipNextSaveRef.current) {
      skipNextSaveRef.current = false;
      return;
    }
    if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    saveTimerRef.current = setTimeout(() => {
      saveTimerRef.current = null;
      const snapshot = hubRef.current;
      void pushHubLayout(snapshot).then((r) => {
        if (r.updatedAt > 0) layoutUpdatedAtRef.current = r.updatedAt;
      });
    }, HUB_SAVE_DEBOUNCE_MS);
    return () => {
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    };
  }, [hub, hydrated]);

  // Multi-device live sync: poll the account store and adopt a newer layout
  // written by another signed-in client (last-write-wins).
  useEffect(() => {
    if (!hydrated) return;
    let cancelled = false;
    const tick = async () => {
      try {
        const res = await fetch("/api/hub/layout", { cache: "no-store" });
        if (!res.ok || cancelled) return;
        const data = (await res.json()) as { layout?: unknown; updatedAt?: number };
        const remoteAt = typeof data.updatedAt === "number" ? data.updatedAt : 0;
        if (remoteAt <= layoutUpdatedAtRef.current) return;
        const layout = sanitizeHubState(data.layout);
        layoutUpdatedAtRef.current = remoteAt;
        writeLocalHubCache(layout);
        skipNextSaveRef.current = true;
        setHub(layout);
      } catch {
        /* offline / transient — next poll retries */
      }
    };
    const id = setInterval(() => void tick(), HUB_SYNC_POLL_MS);
    const onVis = () => {
      if (document.visibilityState === "visible") void tick();
    };
    document.addEventListener("visibilitychange", onVis);
    return () => {
      cancelled = true;
      clearInterval(id);
      document.removeEventListener("visibilitychange", onVis);
    };
  }, [hydrated]);

  // Flush the latest layout on hard close / navigation so other devices see it.
  useEffect(() => {
    if (!hydrated) return;
    const flush = () => {
      if (saveTimerRef.current) {
        clearTimeout(saveTimerRef.current);
        saveTimerRef.current = null;
      }
      const snapshot = hubRef.current;
      writeLocalHubCache(snapshot);
      try {
        void fetch("/api/hub/layout", {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(snapshot),
          keepalive: true,
          credentials: "same-origin",
        });
      } catch {
        /* best-effort */
      }
    };
    window.addEventListener("pagehide", flush);
    return () => window.removeEventListener("pagehide", flush);
  }, [hydrated]);

  // Load the project registry once (feeds the switcher's Recent list).
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const res = await fetch("/api/hub/projects", { cache: "no-store" });
        const data = (await res.json().catch(() => ({}))) as {
          projects?: ProjectEntry[];
          error?: string;
        };
        if (!cancelled && res.ok) setProjects(data.projects ?? []);
        else if (!cancelled) setProjectsError(data.error ?? `load failed (${res.status})`);
      } catch (e) {
        if (!cancelled) setProjectsError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Remember the live session file per project so multi-device reattach works.
  useEffect(() => {
    const ws = agent.state.workspace;
    const file = agent.state.currentSessionFile;
    if (!ws || !file || !file.endsWith(".jsonl")) return;
    setHub((prev) => {
      if (prev.sessions[ws] === file) return prev;
      // Only track sessions for projects we have open as tabs.
      if (!prev.tabPaths.includes(ws) && prev.active !== ws) return prev;
      return { ...prev, sessions: { ...prev.sessions, [ws]: file } };
    });
  }, [agent.state.workspace, agent.state.currentSessionFile]);

  // When the active hub tab changes (local or remote), point the agent at that
  // project's remembered session (or its most-recent via switch_workspace).
  const lastSyncedTabRef = useRef<string | null>(null);
  useEffect(() => {
    if (!hydrated) return;
    const path = hub.active;
    if (!path) return;
    if (lastSyncedTabRef.current === path && agent.state.workspace === path) return;
    lastSyncedTabRef.current = path;
    const remembered = hub.sessions[path];
    void (async () => {
      try {
        if (remembered) {
          await agent.loadSession(remembered, path);
        } else if (agent.state.workspace !== path) {
          await agent.switchWorkspace(path);
        }
      } catch {
        /* errors surface via agent toasts */
      }
    })();
    // Intentionally only react to tab changes — not every session update.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hydrated, hub.active]);

  const activePath = hub.active;

  // ── tab actions ───────────────────────────────────────────────────────────
  const openTab = useCallback(
    async (path: string, name?: string) => {
      const abs = path.trim();
      if (!abs) return;
      setSwitching(true);
      try {
        // Register the project (idempotent) so git/file routes allowlist it.
        const res = await fetch("/api/hub/projects", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ action: "add", path: abs }),
        });
        const data = (await res.json().catch(() => ({}))) as {
          ok?: boolean;
          projects?: ProjectEntry[];
          error?: string;
        };
        if (!res.ok || !data.ok) {
          setProjectsError(data.error ?? `could not open project (${res.status})`);
          return;
        }
        setProjects(data.projects ?? []);
        setProjectsError(null);
        setHub((prev) => {
          const tabPaths = prev.tabPaths.includes(abs)
            ? prev.tabPaths
            : [...prev.tabPaths, abs];
          return {
            ...prev,
            tabPaths,
            active: abs,
            names: { ...prev.names, [abs]: name ?? prev.names[abs] ?? pathBasename(abs) },
          };
        });
        // Drive the agent immediately (don't wait for the effect).
        lastSyncedTabRef.current = abs;
        const remembered = hubRef.current.sessions[abs];
        if (remembered) await agent.loadSession(remembered, abs);
        else await agent.switchWorkspace(abs);
      } finally {
        setSwitching(false);
      }
    },
    [agent],
  );

  const closeTab = useCallback((path: string) => {
    setHub((prev) => {
      const tabPaths = prev.tabPaths.filter((p) => p !== path);
      const active =
        prev.active === path
          ? (tabPaths[Math.max(0, prev.tabPaths.indexOf(path) - 1)] ?? null)
          : prev.active;
      // Keep sessions[path] so reopening the project reattaches the same chat.
      return { ...prev, tabPaths, active };
    });
  }, []);

  const removeProject = useCallback(
    async (path: string) => {
      try {
        const res = await fetch("/api/hub/projects", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ action: "remove", path }),
        });
        const data = (await res.json().catch(() => ({}))) as {
          ok?: boolean;
          projects?: ProjectEntry[];
          error?: string;
        };
        if (res.ok && data.ok) {
          setProjects(data.projects ?? []);
          closeTab(path);
        } else {
          setProjectsError(data.error ?? `could not remove project (${res.status})`);
        }
      } catch (e) {
        setProjectsError(e instanceof Error ? e.message : String(e));
      }
    },
    [closeTab],
  );

  const switcherProjects = useMemo(() => {
    // Merge registry + open tabs so every open tab is listed even if not yet touched.
    const byPath = new Map<string, ProjectEntry>();
    for (const p of projects) byPath.set(p.path, p);
    for (const path of hub.tabPaths) {
      if (!byPath.has(path)) {
        byPath.set(path, {
          path,
          name: hub.names[path] ?? pathBasename(path),
          lastUsed: Date.now(),
        });
      }
    }
    return [...byPath.values()].sort((a, b) => b.lastUsed - a.lastUsed);
  }, [projects, hub.tabPaths, hub.names]);

  const selectTab = useCallback((path: string) => {
    setHub((prev) => (prev.active === path ? prev : { ...prev, active: path }));
  }, []);

  // ── git resize ────────────────────────────────────────────────────────────
  const onGitResizeStart = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault();
      gitResizeRef.current = { startX: e.clientX, startWidth: hub.gitWidth };
      const onMove = (ev: PointerEvent) => {
        const start = gitResizeRef.current;
        if (!start) return;
        // Dragging the left edge of a right sidebar: moving left grows width.
        const next = Math.min(560, Math.max(240, start.startWidth + (start.startX - ev.clientX)));
        setHub((prev) => (prev.gitWidth === next ? prev : { ...prev, gitWidth: next }));
      };
      const onUp = () => {
        gitResizeRef.current = null;
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    },
    [hub.gitWidth],
  );

  // ── keyboard: Ctrl/Cmd+1..9 jump tabs ─────────────────────────────────────
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      if (e.key < "1" || e.key > "9") return;
      const idx = Number(e.key) - 1;
      const path = hubRef.current.tabPaths[idx];
      if (!path) return;
      e.preventDefault();
      selectTab(path);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectTab]);

  const onSignOut = useCallback(async () => {
    setSigningOut(true);
    try {
      // Flush layout so the next device/sign-in sees the latest tabs/sessions.
      if (saveTimerRef.current) {
        clearTimeout(saveTimerRef.current);
        saveTimerRef.current = null;
      }
      await pushHubLayout(hubRef.current);
      await signOut();
      window.location.href = "/login";
    } catch {
      setSigningOut(false);
    }
  }, []);

  // ── IdeContext for Chat + GitPanel ────────────────────────────────────────
  const ideApi = useMemo<IdeApi>(
    () => ({
      state: {
        gitStatus: null, // GitSidebar owns its own status; Chat only reads openTabs.
        openTabs: [],
        activeTabId: null,
      },
      setGitStatus: () => {},
      openDiff: () => {},
      openPatch: () => {},
      openFile: () => {},
      selectEditor: () => {},
      setUiMode: () => {},
    }),
    [],
  );

  // GitSidebar provides its own IdeContext around GitPanel; the shell-level
  // provider is for ChatInner (settings / projects / attach).
  const ideValue = useMemo<IdeContextValue>(
    () => ({
      workspace: activePath ?? agent.state.workspace ?? "",
      ide: ideApi,
      openSettings: () => setSettingsOpen(true),
      openProjects: () => setSwitcherOpen(true),
      attachToChat: (payload) => attachRef.current?.(payload),
      registerAttachToChat: (fn) => {
        attachRef.current = fn;
      },
    }),
    [activePath, agent.state.workspace, ideApi],
  );

  const gitOpenDesktop = hub.gitOpen && !isMobile;
  const showGitDrawer = isMobile && mobileGitOpen;

  if (!hydrated) {
    return (
      <div className="flex h-[100dvh] items-center justify-center bg-ink-950 text-ink-400">
        <div className="flex items-center gap-3 font-mono text-[12px]">
          <BrandMark size={22} />
          <span>Loading workspace…</span>
        </div>
      </div>
    );
  }

  return (
    <IdeContext.Provider value={ideValue}>
      <div className="flex h-[100dvh] w-full flex-col overflow-hidden bg-ink-950 text-ink-100">
        {/* ── Top bar: brand + project tabs + actions ─────────────────────── */}
        <header className="flex h-12 shrink-0 items-center gap-1 border-b border-ink-800/90 bg-ink-925/95 px-2 backdrop-blur-sm">
          <div className="flex items-center gap-2 px-1.5">
            <BrandMark size={18} />
            <span className="hidden font-display text-[13px] font-semibold tracking-tight text-ink-100 sm:inline">
              CatCode
            </span>
          </div>

          <div className="mx-1 h-5 w-px bg-ink-800/80" aria-hidden />

          {/* Project tabs */}
          <div className="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto">
            {hub.tabPaths.map((path, i) => {
              const active = path === activePath;
              const label = hub.names[path] ?? pathBasename(path);
              const live =
                agent.state.liveSessions &&
                Object.values(agent.state.liveSessions).some(
                  (s) => s.workspace === path && (s.streaming || s.needsAttention),
                );
              return (
                <button
                  key={path}
                  type="button"
                  onClick={() => selectTab(path)}
                  className={`group relative flex max-w-[12rem] items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-left text-[12px] transition-colors ${
                    active
                      ? "bg-ink-850 text-ink-100 shadow-[inset_0_0_0_1px_rgb(var(--ink-700)/0.5)]"
                      : "text-ink-400 hover:bg-ink-900/80 hover:text-ink-200"
                  }`}
                  title={`${label}\n${path}${i < 9 ? `\n⌘${i + 1}` : ""}`}
                >
                  <FolderIcon width={12} height={12} className="shrink-0 opacity-70" />
                  <span className="min-w-0 truncate font-medium">{label}</span>
                  {live && (
                    <span
                      className="h-1.5 w-1.5 shrink-0 rounded-full bg-accent shadow-[0_0_6px_rgb(var(--accent)/0.8)]"
                      title="Live session activity"
                    />
                  )}
                  <span
                    role="button"
                    tabIndex={-1}
                    onClick={(e) => {
                      e.stopPropagation();
                      closeTab(path);
                    }}
                    className={`rounded-md p-0.5 text-ink-600 hover:bg-ink-800 hover:text-ink-200 ${
                      isMobile ? "opacity-100" : "opacity-0 group-hover:opacity-100"
                    }`}
                    aria-label={`Close ${label}`}
                  >
                    <XIcon width={11} height={11} />
                  </span>
                </button>
              );
            })}
            <button
              type="button"
              onClick={() => setSwitcherOpen(true)}
              disabled={switching}
              className="focus-ring ml-0.5 flex h-8 w-8 items-center justify-center rounded-lg text-ink-500 transition-colors hover:bg-ink-900 hover:text-ink-200 disabled:opacity-50"
              title="Open project"
              aria-label="Open project"
            >
              <PlusIcon width={14} height={14} />
            </button>
          </div>

          {/* Right actions */}
          <div className="flex shrink-0 items-center gap-0.5">
            <button
              type="button"
              onClick={() => {
                if (isMobile) setMobileGitOpen((o) => !o);
                else setHub((p) => ({ ...p, gitOpen: !p.gitOpen }));
              }}
              className={`flex h-7 items-center gap-1.5 rounded-sm px-2 text-[11px] transition-colors ${
                gitOpenDesktop || showGitDrawer
                  ? "bg-ink-850 text-ink-100"
                  : "text-ink-500 hover:bg-ink-900 hover:text-ink-200"
              }`}
              title="Source control"
              aria-label="Toggle git panel"
              aria-pressed={gitOpenDesktop || showGitDrawer}
            >
              <GitBranchIcon width={13} height={13} />
              <span className="hidden sm:inline">Git</span>
            </button>

            <div className="relative" ref={mergeRefs(menuRef, menuTrapRef)}>
              <button
                type="button"
                onClick={() => setMenuOpen((o) => !o)}
                className="flex h-7 w-7 items-center justify-center rounded-sm text-ink-500 transition-colors hover:bg-ink-900 hover:text-ink-200"
                aria-label="Account menu"
                aria-expanded={menuOpen}
              >
                <UserIcon width={14} height={14} />
              </button>
              {menuOpen && (
                <div
                  className="absolute right-0 top-full z-50 mt-1 min-w-[10rem] overflow-hidden rounded-md border border-ink-750 bg-ink-900 py-1 shadow-elev-2"
                  role="menu"
                >
                  <button
                    type="button"
                    role="menuitem"
                    className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] text-ink-200 hover:bg-ink-850"
                    onClick={() => {
                      setMenuOpen(false);
                      setSettingsOpen(true);
                    }}
                  >
                    Settings
                  </button>
                  <button
                    type="button"
                    role="menuitem"
                    disabled={signingOut}
                    className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] text-ink-200 hover:bg-ink-850 disabled:opacity-50"
                    onClick={() => {
                      setMenuOpen(false);
                      void onSignOut();
                    }}
                  >
                    {signingOut ? "Signing out…" : "Sign out"}
                  </button>
                </div>
              )}
            </div>
          </div>
        </header>

        {projectsError && (
          <div className="shrink-0 border-b border-danger/30 bg-danger/10 px-3 py-1.5 font-mono text-[11px] text-danger">
            {projectsError}
          </div>
        )}

        {/* ── Body: chat + optional git ───────────────────────────────────── */}
        <div className="relative flex min-h-0 flex-1">
          <main className="relative flex min-h-0 min-w-0 flex-1 flex-col">
            {activePath ? (
              <ErrorBoundary label="chat">
                <ChatPane agent={agent} />
              </ErrorBoundary>
            ) : (
              <EmptyHub onOpenProject={() => setSwitcherOpen(true)} />
            )}
          </main>

          {/* Desktop git column */}
          {gitOpenDesktop && activePath && (
            <>
              <div
                role="separator"
                aria-orientation="vertical"
                aria-label="Resize git panel"
                onPointerDown={onGitResizeStart}
                className="w-1.5 shrink-0 cursor-col-resize touch-none bg-transparent hover:bg-accent/30 active:bg-accent/50"
              />
              <aside
                className="flex min-h-0 shrink-0 flex-col border-l border-ink-800/80 bg-ink-925"
                style={{ width: hub.gitWidth }}
              >
                <ErrorBoundary label="git">
                  <GitSidebar workspace={activePath} />
                </ErrorBoundary>
              </aside>
            </>
          )}

          {/* Mobile git drawer */}
          {showGitDrawer && activePath && (
            <>
              <div
                className="absolute inset-0 z-40 bg-black/50"
                onClick={() => setMobileGitOpen(false)}
                aria-hidden
              />
              <aside
                ref={mergeRefs(mobileGitCloseRef, mobileGitTrapRef)}
                className="absolute inset-y-0 right-0 z-50 flex w-[min(100%,22rem)] flex-col border-l border-ink-800 bg-ink-925 shadow-elev-2"
                role="dialog"
                aria-modal="true"
                aria-label="Source control"
              >
                <div className="flex h-10 items-center justify-between border-b border-ink-800 px-3">
                  <span className="text-[11px] font-semibold uppercase tracking-wider text-ink-400">
                    Source Control
                  </span>
                  <button
                    type="button"
                    onClick={() => setMobileGitOpen(false)}
                    className="rounded p-1 text-ink-500 hover:bg-ink-850 hover:text-ink-100"
                    aria-label="Close git panel"
                  >
                    <XIcon width={14} height={14} />
                  </button>
                </div>
                <div className="min-h-0 flex-1">
                  <ErrorBoundary label="git-mobile">
                    <GitSidebar workspace={activePath} />
                  </ErrorBoundary>
                </div>
              </aside>
            </>
          )}
        </div>

        {switcherOpen && (
          <ProjectSwitcher
            workspace={activePath ?? ""}
            projects={switcherProjects}
            switching={switching}
            mobile={isMobile}
            onSwitchWorkspace={(path) => {
              setSwitcherOpen(false);
              void openTab(path);
            }}
            onRemoveProject={(path) => void removeProject(path)}
            onClose={() => setSwitcherOpen(false)}
          />
        )}

        {settingsOpen && (
          <SettingsModal
            ready={agent.state.ready}
            models={agent.state.models}
            selectedModel={agent.state.selectedModel}
            modelsRefreshing={agent.state.modelsRefreshing}
            thinkingLevel={agent.state.thinkingLevel}
            approvalMode={agent.state.approvalMode}
            autoCompact={agent.state.ready?.auto_compact ?? true}
            sandbox={agent.state.ready?.sandbox ?? "none"}
            onSelectModel={agent.setModel}
            onRefreshModels={() => void agent.refreshModels()}
            onSelectThinking={agent.setThinking}
            onSetApproval={agent.setApproval}
            onSetBashTimeout={(secs) => void agent.setConfig("bash_timeout_secs", secs)}
            onSetAutoCompact={(on) => void agent.setConfig("auto_compact", on)}
            onSetSandbox={(mode) => void agent.setConfig("sandbox", mode)}
            sandboxStatus={agent.state.sandbox}
            onRecheckSandbox={() => void agent.getSandboxStatus()}
            onPrepareSandbox={() => void agent.prepareSandbox()}
            onResetSandbox={() => void agent.resetSandbox()}
            visionConfig={agent.state.visionConfig}
            onSetVisionConfig={(vision_model, vision_models, enabled) =>
              void agent.setVisionConfig(vision_model, vision_models, enabled)
            }
            onRefreshVision={() => void agent.getVisionConfig()}
            onClose={() => setSettingsOpen(false)}
            uiMode="chat"
          />
        )}
      </div>
    </IdeContext.Provider>
  );
}

/** ChatInner needs the agent from the parent (one hook per shell). */
function ChatPane({ agent }: { agent: ReturnType<typeof useAgent> }) {
  return <ChatInner agent={agent} docked />;
}

function EmptyHub({ onOpenProject }: { onOpenProject: () => void }) {
  return (
    <div className="chat-empty flex h-full items-center justify-center px-6 py-10">
      <div className="w-full max-w-md border-l border-ink-800 pl-5 text-left sm:pl-6">
        <div className="flex items-center gap-3">
          <BrandMark size={24} />
          <p className="font-mono text-[10px] uppercase tracking-[0.14em] text-ink-500">
            No project open
          </p>
        </div>
        <h1 className="mt-5 font-display text-xl font-semibold text-ink-100">
          Choose a repository to start work
        </h1>
        <p className="mt-2 max-w-sm text-[13px] leading-relaxed text-ink-400">
          Open a local project to start a session and keep its chat and repository state together.
        </p>
        <button
          type="button"
          onClick={onOpenProject}
          className="focus-ring mt-6 inline-flex items-center gap-2 rounded-md bg-accent px-3.5 py-2.5 text-[13px] font-semibold text-white transition-colors hover:bg-accent-soft"
        >
          <FolderPlusIcon width={15} height={15} />
          Open project
        </button>
      </div>
    </div>
  );
}
