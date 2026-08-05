"use client";

// The /hub shell — a project-centric terminal workspace:
//
//   • PROJECT TABS across the top. Add projects by browsing the machine
//     (reuses the IDE's ProjectSwitcher → /api/browse), or create/clone them.
//   • GIT SIDEBAR (collapsible, resizable) — the IDE's full GitPanel bound
//     to the active project via an IdeContext shim (see ./git-sidebar.tsx).
//   • SPLIT TERMINAL GRID — every pane is a persistent PTY that auto-runs
//     `catcode` in the project root. Split right/down per pane, or apply a
//     preset (1, 1×2, 2×1, 2×2, 3×3, 4×4). Dividers drag; ratios persist.
//   • Tab-switching KEEPS every project mounted (display:none) so terminals
//     never detach; PTYs are server-persistent anyway (see server.ts).
//
// Pane ids double as server-side terminal session ids, so a refresh reattaches
// every running catcode (scrollback replayed by the WS endpoint).

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  BrandMark,
  FolderIcon,
  FolderPlusIcon,
  GitBranchIcon,
  PlusIcon,
  TerminalIcon,
  XIcon,
} from "@/components/icons";
import { ProjectSwitcher } from "@/components/ide/project-switcher";
import {
  HUB_PRESETS,
  MAX_PANES,
  closeLeaf,
  countLeaves,
  firstLeafId,
  gridLayoutWithIds,
  leafIds,
  leafNode,
  presetShape,
  replaceLeafId,
  setRatio,
  splitLeaf,
  type HubPreset,
  type LayoutNode,
} from "@/lib/hub-layout";
import type { ProjectEntry } from "@/lib/types";
import { HubGitSidebar as GitSidebar } from "./git-sidebar";
import { HubPane } from "./pane";
import { SplitView } from "./split-view";
import {
  defaultHubState,
  loadHubState,
  newPaneId,
  pathBasename,
  saveHubState,
  type HubPersistState,
} from "./hub-state";
import { terminateTerminalSession } from "@/components/ide/terminal";

export function HubShell() {
  // ── state ─────────────────────────────────────────────────────────────────
  const [hub, setHub] = useState<HubPersistState>(defaultHubState);
  const [hydrated, setHydrated] = useState(false);
  const [projects, setProjects] = useState<ProjectEntry[]>([]);
  const [switcherOpen, setSwitcherOpen] = useState(false);
  const [switching, setSwitching] = useState(false);
  const [projectsError, setProjectsError] = useState<string | null>(null);
  const gitResizeRef = useRef<{ startX: number; startWidth: number } | null>(null);

  // Restore persisted tabs/layouts after hydration (server render is a splash).
  useEffect(() => {
    setHub(loadHubState());
    setHydrated(true);
  }, []);

  // Persist on every change (post-hydration only).
  useEffect(() => {
    if (hydrated) saveHubState(hub);
  }, [hub, hydrated]);

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

  const activePath = hub.active;
  const activeLayout = activePath ? (hub.layouts[activePath] ?? null) : null;

  // ── tab actions ───────────────────────────────────────────────────────────
  const openTab = useCallback(async (path: string, name?: string) => {
    const abs = path.trim();
    if (!abs) return;
    setSwitching(true);
    try {
      // Register the project (idempotent) so the terminal WS + git routes
      // allowlist the workspace immediately.
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
          layouts: { ...prev.layouts, [abs]: prev.layouts[abs] ?? leafNode(newPaneId()) },
        };
      });
    } finally {
      setSwitching(false);
    }
  }, []);

  const closeTab = useCallback(
    (path: string) => {
      setHub((prev) => {
        // Terminate every PTY the tab owns (layout is kept so reopening the
        // project restores the arrangement with fresh catcode instances).
        const layout = prev.layouts[path];
        if (layout) for (const id of leafIds(layout)) terminateTerminalSession(id, path);
        const tabPaths = prev.tabPaths.filter((p) => p !== path);
        const active =
          prev.active === path
            ? (tabPaths[Math.max(0, prev.tabPaths.indexOf(path) - 1)] ?? null)
            : prev.active;
        return { ...prev, tabPaths, active };
      });
    },
    [],
  );

  // ── pane actions (active project) ────────────────────────────────────────
  const updateLayout = useCallback(
    (updater: (layout: LayoutNode) => LayoutNode | null) => {
      if (!activePath) return;
      setHub((prev) => {
        const layout = prev.layouts[activePath];
        if (!layout) return prev;
        const next = updater(layout);
        if (next === null) return prev;
        return { ...prev, layouts: { ...prev.layouts, [activePath]: next } };
      });
    },
    [activePath],
  );

  const splitPane = useCallback(
    (paneId: string, dir: "h" | "v") => {
      if (!activePath) return;
      const newId = newPaneId();
      setHub((prev) => {
        const layout = prev.layouts[activePath];
        if (!layout) return prev;
        // Returns null when the MAX_PANES cap is hit → state left untouched.
        const next = splitLeaf(layout, paneId, dir, newId);
        if (!next) return prev;
        return {
          ...prev,
          layouts: { ...prev.layouts, [activePath]: next },
          focused: { ...prev.focused, [activePath]: newId },
        };
      });
    },
    [activePath],
  );

  const closePane = useCallback(
    (paneId: string) => {
      if (!activePath) return;
      const layout = hub.layouts[activePath];
      if (!layout) return;
      const ok =
        countLeaves(layout) <= 1 ||
        window.confirm("Close this pane? Its catcode session will be terminated.");
      if (!ok) return;
      terminateTerminalSession(paneId, activePath);
      setHub((prev) => {
        const current = prev.layouts[activePath];
        if (!current) return prev;
        const next = closeLeaf(current, paneId) ?? leafNode(newPaneId());
        const focused = firstLeafId(next);
        return {
          ...prev,
          layouts: { ...prev.layouts, [activePath]: next },
          focused: { ...prev.focused, [activePath]: focused },
        };
      });
    },
    [activePath, hub.layouts],
  );

  const restartPane = useCallback(
    (paneId: string) => {
      if (!activePath) return;
      terminateTerminalSession(paneId, activePath);
      const newId = newPaneId();
      updateLayout((layout) => replaceLeafId(layout, paneId, newId) ?? layout);
    },
    [activePath, updateLayout],
  );

  const focusPane = useCallback(
    (paneId: string) => {
      if (!activePath) return;
      setHub((prev) => ({ ...prev, focused: { ...prev.focused, [activePath]: paneId } }));
    },
    [activePath],
  );

  const applyPreset = useCallback(
    (preset: HubPreset) => {
      if (!activePath) return;
      const { rows, cols } = presetShape(preset);
      setHub((prev) => {
        const layout = prev.layouts[activePath];
        if (!layout) return prev;
        const existing = leafIds(layout);
        const need = rows * cols;
        // Reuse existing pane ids (their PTYs keep running) and mint ids for
        // the shortfall; terminate anything beyond the new grid.
        const ids = [...existing];
        while (ids.length < need) ids.push(newPaneId());
        const kept = ids.slice(0, need);
        for (const id of ids.slice(need)) terminateTerminalSession(id, activePath);
        const next = gridLayoutWithIds(rows, cols, kept);
        const focusedId = prev.focused[activePath];
        const focused =
          focusedId && kept.includes(focusedId) ? focusedId : firstLeafId(next);
        return {
          ...prev,
          layouts: { ...prev.layouts, [activePath]: next },
          focused: { ...prev.focused, [activePath]: focused },
        };
      });
    },
    [activePath],
  );

  const onRatioChange = useCallback(
    (splitId: string, ratio: number) => {
      updateLayout((layout) => setRatio(layout, splitId, ratio));
    },
    [updateLayout],
  );

  // ── project removal (from the switcher's Recent list) ────────────────────
  const removeProject = useCallback(async (path: string) => {
    try {
      const res = await fetch("/api/hub/projects", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action: "remove", path }),
      });
      const data = (await res.json().catch(() => ({}))) as {
        ok?: boolean;
        projects?: ProjectEntry[];
      };
      if (res.ok && data.ok) setProjects(data.projects ?? []);
    } catch {
      /* best-effort */
    }
  }, []);

  // ── keyboard: Ctrl/Cmd+1..9 switches project tabs ────────────────────────
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return;
      const n = Number(e.key);
      if (!Number.isInteger(n) || n < 1 || n > 9) return;
      setHub((prev) => {
        const path = prev.tabPaths[n - 1];
        return path ? { ...prev, active: path } : prev;
      });
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // ── git sidebar resize ────────────────────────────────────────────────────
  const startGitResize = (e: React.PointerEvent) => {
    gitResizeRef.current = { startX: e.clientX, startWidth: hub.gitWidth };
    const move = (ev: PointerEvent) => {
      const start = gitResizeRef.current;
      if (!start) return;
      const width = Math.min(560, Math.max(240, start.startWidth + (start.startX - ev.clientX)));
      setHub((prev) => ({ ...prev, gitWidth: width }));
    };
    const up = () => {
      gitResizeRef.current = null;
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  // The switcher renders the CURRENT project as "active" — include open tabs
  // that may no longer be in projects.json (e.g. removed from Recent).
  const switcherProjects = useMemo(() => {
    const byPath = new Map(projects.map((p) => [p.path, p]));
    for (const path of hub.tabPaths) {
      if (!byPath.has(path)) {
        byPath.set(path, { path, name: hub.names[path] ?? pathBasename(path), lastUsed: Date.now() });
      }
    }
    return [...byPath.values()];
  }, [projects, hub.tabPaths, hub.names]);

  // ── render ────────────────────────────────────────────────────────────────
  if (!hydrated) {
    return (
      <div className="flex h-dvh items-center justify-center bg-ink-950 text-ink-500">
        <div className="flex items-center gap-2 text-sm">
          <BrandMark size={22} />
          Loading hub…
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-dvh flex-col overflow-hidden bg-ink-950 text-ink-100">
      {/* ── tab bar ── */}
      <header className="flex shrink-0 items-center gap-1 border-b border-ink-800 bg-ink-900/70 px-2 pt-1.5">
        <span className="mr-1 flex items-center gap-1.5 pr-1" title="Catalyst Code Hub">
          <BrandMark size={20} />
          <span className="hidden text-[12px] font-semibold tracking-wide text-ink-300 sm:inline">
            Hub
          </span>
        </span>

        <div
          className="flex min-w-0 flex-1 items-end gap-0.5 overflow-x-auto"
          role="tablist"
          aria-label="Projects"
        >
          {hub.tabPaths.map((path) => {
            const active = path === activePath;
            const name = hub.names[path] ?? pathBasename(path);
            return (
              <div
                key={path}
                role="tab"
                aria-selected={active}
                title={path}
                onClick={() => setHub((prev) => ({ ...prev, active: path }))}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    setHub((prev) => ({ ...prev, active: path }));
                  }
                }}
                tabIndex={0}
                className={`group/tab flex max-w-[16rem] shrink-0 cursor-pointer items-center gap-1.5 rounded-t-lg border border-b-0 px-3 py-1.5 text-[12px] transition-colors ${
                  active
                    ? "border-ink-700 bg-ink-950 text-ink-100"
                    : "border-transparent text-ink-400 hover:bg-ink-850 hover:text-ink-200"
                }`}
              >
                <FolderIcon width={12} height={12} className={active ? "text-accent-soft" : "text-ink-600"} />
                <span className="max-w-[11rem] truncate font-medium">{name}</span>
                <button
                  type="button"
                  title={`Close ${name}`}
                  aria-label={`Close ${name}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    closeTab(path);
                  }}
                  className="rounded p-0.5 text-ink-600 opacity-0 transition-opacity hover:bg-danger/15 hover:text-danger group-hover/tab:opacity-100 focus:opacity-100"
                >
                  <XIcon width={11} height={11} />
                </button>
              </div>
            );
          })}

          <button
            type="button"
            onClick={() => setSwitcherOpen(true)}
            title="Add or switch project"
            aria-label="Add or switch project"
            className="ml-0.5 shrink-0 rounded-md p-1.5 text-ink-400 transition-colors hover:bg-ink-850 hover:text-ink-100"
          >
            <PlusIcon width={14} height={14} />
          </button>
        </div>

        {/* ── layout presets (active project) ── */}
        {activePath ? (
          <div
            className="hidden items-center gap-0.5 rounded-lg border border-ink-800 bg-ink-950 p-0.5 md:flex"
            role="group"
            aria-label="Terminal layout presets"
            title="Terminal layout"
          >
            {HUB_PRESETS.map((preset) => {
              const paneCount = activeLayout ? countLeaves(activeLayout) : 1;
              const current = paneCount === preset.rows * preset.cols;
              return (
                <button
                  key={preset.id}
                  type="button"
                  onClick={() => applyPreset(preset.id)}
                  disabled={preset.rows * preset.cols > MAX_PANES}
                  title={`${preset.rows}×${preset.cols} terminals`}
                  aria-label={`Layout ${preset.label}`}
                  aria-pressed={current}
                  className={`rounded px-1.5 py-1 font-mono text-[10px] transition-colors ${
                    current
                      ? "bg-accent/15 text-accent-soft"
                      : "text-ink-500 hover:bg-ink-850 hover:text-ink-200"
                  } disabled:opacity-30`}
                >
                  {preset.label}
                </button>
              );
            })}
          </div>
        ) : null}

        <button
          type="button"
          onClick={() => setHub((prev) => ({ ...prev, gitOpen: !prev.gitOpen }))}
          title={hub.gitOpen ? "Hide Git panel" : "Show Git panel"}
          aria-label={hub.gitOpen ? "Hide Git panel" : "Show Git panel"}
          aria-pressed={hub.gitOpen}
          className={`shrink-0 rounded-md p-1.5 transition-colors hover:bg-ink-850 ${
            hub.gitOpen ? "text-accent-soft" : "text-ink-400 hover:text-ink-100"
          }`}
        >
          <GitBranchIcon width={15} height={15} />
        </button>
      </header>

      {projectsError ? (
        <div className="flex shrink-0 items-center gap-2 border-b border-ink-800 bg-danger/10 px-3 py-1 text-[11px] text-danger">
          <span className="min-w-0 flex-1 truncate">{projectsError}</span>
          <button
            type="button"
            className="rounded p-0.5 hover:bg-danger/20"
            aria-label="Dismiss error"
            onClick={() => setProjectsError(null)}
          >
            <XIcon width={11} height={11} />
          </button>
        </div>
      ) : null}

      {/* ── body ── */}
      <div className="flex min-h-0 flex-1">
        {/* Every tab stays mounted; inactive ones are display:none so their
            terminals never detach while switching projects. */}
        {hub.tabPaths.map((path) => (
          <main
            key={path}
            className={`min-h-0 min-w-0 flex-1 ${path === activePath ? "" : "hidden"}`}
            role="tabpanel"
            aria-label={hub.names[path] ?? pathBasename(path)}
          >
            {hub.layouts[path] ? (
              <SplitView
                node={hub.layouts[path]}
                onRatioChange={path === activePath ? onRatioChange : () => {}}
                renderLeaf={(leafId) => (
                  <HubPane
                    paneId={leafId}
                    workspace={path}
                    focused={path === activePath && (hub.focused[path] ?? "") === leafId}
                    onFocus={() => (path === activePath ? focusPane(leafId) : undefined)}
                    onSplitRight={() => splitPane(leafId, "h")}
                    onSplitDown={() => splitPane(leafId, "v")}
                    onRestart={() => restartPane(leafId)}
                    onClose={() => closePane(leafId)}
                  />
                )}
              />
            ) : null}
          </main>
        ))}

        {hub.tabPaths.length === 0 ? <EmptyState onAdd={() => setSwitcherOpen(true)} switching={switching} /> : null}

        {/* ── git sidebar ── */}
        {activePath && hub.gitOpen ? (
          <>
            <div
              role="separator"
              aria-orientation="vertical"
              onPointerDown={startGitResize}
              className="w-1.5 shrink-0 cursor-col-resize bg-ink-800/70 transition-colors hover:bg-accent/60"
              title="Drag to resize the Git panel"
            />
            <aside
              className="shrink-0 border-l border-ink-800 bg-ink-925"
              style={{ width: hub.gitWidth }}
              aria-label="Git"
            >
              <GitSidebar workspace={activePath} />
            </aside>
          </>
        ) : null}
      </div>

      {/* ── project switcher (recent / browse / create / clone) ── */}
      {switcherOpen ? (
        <ProjectSwitcher
          workspace={activePath ?? ""}
          projects={switcherProjects}
          switching={switching}
          onSwitchWorkspace={(path) => {
            void openTab(path);
          }}
          onRemoveProject={(path) => void removeProject(path)}
          onClose={() => setSwitcherOpen(false)}
        />
      ) : null}
    </div>
  );
}

function EmptyState({ onAdd, switching }: { onAdd: () => void; switching: boolean }) {
  return (
    <main className="flex min-h-0 min-w-0 flex-1 items-center justify-center p-6" role="tabpanel">
      <div className="max-w-md text-center">
        <div className="mx-auto mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-accent/10 ring-1 ring-accent/25">
          <TerminalIcon width={24} height={24} className="text-accent-soft" />
        </div>
        <h1 className="text-[15px] font-semibold text-ink-100">No projects open yet</h1>
        <p className="mt-2 text-[12px] leading-relaxed text-ink-400">
          Add a project by browsing the files on this machine. Each project gets its own tab with
          a Git panel and a grid of terminals — every terminal launches{" "}
          <code className="rounded bg-ink-900 px-1 py-0.5 font-mono text-[11px] text-accent-soft">catcode</code>{" "}
          in the project root automatically.
        </p>
        <button
          type="button"
          onClick={onAdd}
          disabled={switching}
          className="mt-5 inline-flex items-center gap-2 rounded-lg bg-accent px-4 py-2 text-[12px] font-semibold text-white transition-colors hover:bg-accent-soft disabled:opacity-50"
        >
          <FolderPlusIcon width={14} height={14} />
          {switching ? "Opening…" : "Add a project"}
        </button>
      </div>
    </main>
  );
}
