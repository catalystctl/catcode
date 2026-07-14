"use client";

import { useMemo, useState } from "react";
import { useAgent } from "@/lib/use-agent";
import { useIde } from "@/lib/use-ide";
import { IdeContext, useIdeContext } from "@/lib/ide-context";
import { Chat } from "@/components/chat";
import type {
  DockPosition,
  GitStatus,
  MovablePanelId,
} from "@/lib/types";
import {
  FileTree,
  GitPanel,
  Editor,
  TerminalPanel,
  Preview,
} from "./panel-registry";
import { ActivityBar } from "./activity-bar";
import { ResizeHandle } from "./resize-handle";
import {
  FileIcon,
  GitBranchIcon,
  XIcon,
  ChevronRight,
} from "@/components/icons";

const MOVABLE: MovablePanelId[] = ["chat", "terminal", "git", "preview"];
const LABELS: Record<MovablePanelId, string> = {
  chat: "AI Chat",
  terminal: "Terminal",
  git: "Source Control",
  preview: "Preview",
};

export function IdeShell() {
  const agent = useAgent();
  const ide = useIde();
  const workspace = agent.state.workspace;
  const [dragging, setDragging] = useState<MovablePanelId | null>(null);
  const ctx = useMemo(() => ({ workspace, ide }), [workspace, ide]);

  const drop = (position: DockPosition, payload?: string) => {
    const panel = MOVABLE.includes(payload as MovablePanelId)
      ? payload as MovablePanelId
      : dragging;
    if (panel) ide.movePanel(panel, position);
    setDragging(null);
  };

  return (
    <IdeContext.Provider value={ctx}>
      <div className="relative flex h-[100dvh] w-full overflow-hidden bg-ink-950 text-ink-100">
        <ActivityBar />

        {!ide.state.sidebarCollapsed && (
          <>
            <PrimarySidebar
              workspace={workspace}
              agent={agent}
              onDragStart={setDragging}
              onDragEnd={() => setDragging(null)}
            />
            <ResizeHandle
              orientation="x"
              size={ide.state.sidebarWidth}
              onResize={ide.setSidebarWidth}
              min={160}
              max={640}
            />
          </>
        )}

        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex min-h-0 flex-1">
            <main className="flex min-w-0 flex-1 flex-col">
              <TabStrip
                onDragStart={setDragging}
                onDragEnd={() => setDragging(null)}
              />
              <div className="relative min-h-0 flex-1 overflow-hidden bg-ink-950">
                {activeMainPanel(ide) ? (
                  <PanelContent
                    panel={activeMainPanel(ide)!}
                    workspace={workspace}
                    agent={agent}
                  />
                ) : (
                  <MainContent workspace={workspace} />
                )}
              </div>
            </main>

            {hasVisibleDock(ide, "right") && (
              <ResizeHandle
                orientation="x"
                invert
                size={ide.state.copilotWidth}
                onResize={ide.setCopilotWidth}
                min={280}
                max={900}
              />
            )}
            <DockAt
              position="right"
              workspace={workspace}
              agent={agent}
              onDragStart={setDragging}
              onDragEnd={() => setDragging(null)}
            />
          </div>

          {hasVisibleDock(ide, "bottom") && (
            <ResizeHandle
              orientation="y"
              invert
              size={ide.state.bottomPanelHeight}
              onResize={ide.setBottomPanelHeight}
              min={120}
              max={800}
            />
          )}
          <DockAt
            position="bottom"
            workspace={workspace}
            agent={agent}
            onDragStart={setDragging}
            onDragEnd={() => setDragging(null)}
          />

          <StatusBar
            connected={agent.connected}
            workspace={workspace}
            git={ide.state.gitStatus}
          />
        </div>

        {dragging && <DockDropOverlay panel={dragging} onDrop={drop} />}
      </div>
    </IdeContext.Provider>
  );
}

function PrimarySidebar({
  workspace,
  agent,
  onDragStart,
  onDragEnd,
}: {
  workspace: string;
  agent: ReturnType<typeof useAgent>;
  onDragStart: (panel: MovablePanelId) => void;
  onDragEnd: () => void;
}) {
  const { ide } = useIdeContext();
  const panels = MOVABLE.filter(
    (panel) => ide.state.panelVisibility[panel] && ide.state.panelLocations[panel] === "left",
  );
  const requested = ide.state.activeDockPanels.left;
  const active = requested && panels.includes(requested) ? requested : null;

  return (
    <aside
      style={{ width: ide.state.sidebarWidth }}
      className="flex shrink-0 flex-col border-r border-ink-800 bg-ink-925"
    >
      <div className="flex h-9 shrink-0 items-stretch overflow-x-auto border-b border-ink-800">
        <button
          type="button"
          onClick={ide.selectExplorer}
          className={`shrink-0 border-r border-ink-800 px-3 text-[11px] font-semibold uppercase tracking-wide ${active === null ? "bg-ink-950 text-ink-200" : "text-ink-500 hover:bg-ink-900"}`}
        >
          Explorer
        </button>
        {panels.map((panel) => (
          <button
            key={panel}
            type="button"
            draggable
            onDragStart={(event) => {
              event.dataTransfer.effectAllowed = "move";
              event.dataTransfer.setData("text/plain", panel);
              try {
                event.dataTransfer.setData("application/x-catalyst-panel", panel);
              } catch {
                // Some WebKit builds only accept standard text payloads.
              }
              onDragStart(panel);
            }}
            onDragEnd={onDragEnd}
            onClick={() => ide.selectDockPanel("left", panel)}
            className={`group flex min-w-0 items-center gap-1.5 border-r border-ink-800 px-2 text-xs ${active === panel ? "bg-ink-950 text-ink-100" : "text-ink-400 hover:bg-ink-900"}`}
            title={`Drag ${LABELS[panel]} to another dock`}
          >
            <span className="cursor-grab select-none text-ink-600">⠿</span>
            <span className="truncate">{LABELS[panel]}</span>
            <span
              role="button"
              tabIndex={0}
              onClick={(event) => { event.stopPropagation(); ide.hideDockPanel(panel); }}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") ide.hideDockPanel(panel);
              }}
              className="rounded p-0.5 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
              aria-label={`Close ${LABELS[panel]}`}
            >
              <XIcon width={12} height={12} />
            </span>
          </button>
        ))}
        <button
          type="button"
          onClick={ide.toggleSidebar}
          title="Collapse sidebar"
          aria-label="Collapse sidebar"
          className="ml-auto shrink-0 px-2 text-ink-500 hover:text-ink-100"
        >
          <ChevronRight width={14} height={14} />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-hidden">
        {active ? <PanelContent panel={active} workspace={workspace} agent={agent} /> : <FileTree />}
      </div>
    </aside>
  );
}

function hasVisibleDock(ide: ReturnType<typeof useIde>, position: DockPosition) {
  return MOVABLE.some(
    (panel) => ide.state.panelVisibility[panel] && ide.state.panelLocations[panel] === position,
  );
}

function activeMainPanel(ide: ReturnType<typeof useIde>): MovablePanelId | null {
  const panel = ide.state.activeDockPanels.main;
  return panel &&
    ide.state.panelVisibility[panel] &&
    ide.state.panelLocations[panel] === "main"
    ? panel
    : null;
}

function DockAt({
  position,
  workspace,
  agent,
  onDragStart,
  onDragEnd,
}: {
  position: DockPosition;
  workspace: string;
  agent: ReturnType<typeof useAgent>;
  onDragStart: (panel: MovablePanelId) => void;
  onDragEnd: () => void;
}) {
  const { ide } = useIdeContext();
  const panels = MOVABLE.filter(
    (panel) => ide.state.panelVisibility[panel] && ide.state.panelLocations[panel] === position,
  );
  if (panels.length === 0) return null;
  const requested = ide.state.activeDockPanels[position];
  const active = requested && panels.includes(requested) ? requested : panels[0];
  const style =
    position === "left"
      ? { width: ide.state.leftDockWidth }
      : position === "right"
        ? { width: ide.state.copilotWidth }
        : position === "bottom"
          ? { height: ide.state.bottomPanelHeight }
          : undefined;
  const border =
    position === "left" ? "border-r" : position === "right" ? "border-l" : position === "bottom" ? "border-t" : "";

  return (
    <section
      style={style}
      className={`flex min-h-0 min-w-0 shrink-0 flex-col overflow-hidden border-ink-800 bg-ink-950 ${border} ${position === "main" ? "h-full w-full" : ""}`}
      aria-label={`${position} dock`}
    >
      <div className="flex h-9 shrink-0 items-stretch overflow-x-auto border-b border-ink-800 bg-ink-925">
        {panels.map((panel) => (
          <button
            key={panel}
            type="button"
            draggable
            onDragStart={(event) => {
              event.dataTransfer.effectAllowed = "move";
              event.dataTransfer.setData("text/plain", panel);
              try {
                event.dataTransfer.setData("application/x-catalyst-panel", panel);
              } catch {
                // text/plain remains available as the cross-browser fallback.
              }
              onDragStart(panel);
            }}
            onDragEnd={onDragEnd}
            onClick={() => ide.selectDockPanel(position, panel)}
            title={`Drag ${LABELS[panel]} to another dock`}
            className={`group flex min-w-0 items-center gap-2 border-r border-ink-800 px-3 text-xs ${
              panel === active ? "bg-ink-950 text-ink-100" : "text-ink-400 hover:bg-ink-900"
            }`}
          >
            <span className="cursor-grab select-none text-ink-600 group-active:cursor-grabbing">⠿</span>
            <span className="truncate">{LABELS[panel]}</span>
            <span
              role="button"
              tabIndex={0}
              onClick={(event) => {
                event.stopPropagation();
                ide.hideDockPanel(panel);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") ide.hideDockPanel(panel);
              }}
              className="ml-auto rounded p-0.5 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
              aria-label={`Close ${LABELS[panel]}`}
            >
              <XIcon width={12} height={12} />
            </span>
          </button>
        ))}
      </div>
      <div className="min-h-0 min-w-0 flex-1 overflow-hidden">
        <PanelContent panel={active} workspace={workspace} agent={agent} />
      </div>
    </section>
  );
}

function PanelContent({
  panel,
  workspace,
  agent,
}: {
  panel: MovablePanelId;
  workspace: string;
  agent: ReturnType<typeof useAgent>;
}) {
  const { ide } = useIdeContext();
  if (panel === "chat") return <Chat agent={agent} docked />;
  if (panel === "git") return <GitPanel />;
  if (panel === "preview") {
    return <Preview workspace={workspace} preview={ide.state.preview} onPreviewChange={ide.setPreview} />;
  }
  return (
    <TerminalPanel
      workspace={workspace}
      sessions={ide.state.terminals}
      activeId={ide.state.activeTerminalId}
      onNew={() => ide.newTerminal()}
      onClose={ide.closeTerminal}
      onSelect={ide.setActiveTerminal}
      onExit={ide.setTerminalExit}
    />
  );
}

function DockDropOverlay({
  panel,
  onDrop,
}: {
  panel: MovablePanelId;
  onDrop: (position: DockPosition, payload?: string) => void;
}) {
  const target = (position: DockPosition, classes: string) => (
    <div
      onDragOver={(event) => {
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
      }}
      onDrop={(event) => {
        event.preventDefault();
        const payload =
          event.dataTransfer.getData("application/x-catalyst-panel") ||
          event.dataTransfer.getData("text/plain");
        onDrop(position, payload);
      }}
      className={`absolute flex items-center justify-center rounded-xl border-2 border-dashed border-accent/70 bg-accent/15 text-xs font-semibold uppercase tracking-wider text-accent-soft shadow-2xl backdrop-blur-sm ${classes}`}
    >
      Dock {LABELS[panel]} {position === "main" ? "in editor area" : position}
    </div>
  );
  return (
    <div className="pointer-events-none absolute inset-0 z-50 bg-black/20">
      {target("left", "pointer-events-auto bottom-24 left-16 top-16 w-[18%]")}
      {target("right", "pointer-events-auto bottom-24 right-4 top-16 w-[18%]")}
      {target("bottom", "pointer-events-auto bottom-8 left-[22%] right-[22%] h-[22%]")}
      {target("main", "pointer-events-auto bottom-[28%] left-[28%] right-[28%] top-[22%]")}
    </div>
  );
}

function TabStrip({
  onDragStart,
  onDragEnd,
}: {
  onDragStart: (panel: MovablePanelId) => void;
  onDragEnd: () => void;
}) {
  const { ide } = useIdeContext();
  const { openTabs, activeTabId } = ide.state;
  const panelTabs = MOVABLE.filter(
    (panel) => ide.state.panelVisibility[panel] && ide.state.panelLocations[panel] === "main",
  );
  const activePanel = activeMainPanel(ide);
  return (
    <div className="flex h-9 shrink-0 items-stretch overflow-x-auto border-b border-ink-800 bg-ink-925">
      {openTabs.length === 0 && panelTabs.length === 0 && <span className="flex items-center px-3 text-xs text-ink-600">No open editors</span>}
      {openTabs.map((tab) => {
        const active = activePanel === null && tab.id === activeTabId;
        return (
          <button
            key={tab.id}
            type="button"
            onClick={() => ide.setActiveTab(tab.id)}
            title={tab.target}
            className={`group flex items-center gap-1.5 border-r border-ink-800 px-3 text-xs ${active ? "bg-ink-950 text-ink-100" : "text-ink-400 hover:bg-ink-900"}`}
          >
            <FileIcon width={13} height={13} className="shrink-0 text-ink-500" />
            <span className="max-w-[12rem] truncate">{tab.label}</span>
            {tab.dirty && <span className="text-amber-300">●</span>}
            <span
              role="button"
              tabIndex={0}
              onClick={(event) => { event.stopPropagation(); ide.closeTab(tab.id); }}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") { event.preventDefault(); event.stopPropagation(); ide.closeTab(tab.id); }
              }}
              className="ml-1 text-ink-500 opacity-0 hover:text-ink-100 group-hover:opacity-100"
              aria-label={`close ${tab.label}`}
            >
              <XIcon width={12} height={12} />
            </span>
          </button>
        );
      })}
      {panelTabs.map((panel) => (
        <button
          key={`panel:${panel}`}
          type="button"
          draggable
          onDragStart={(event) => {
            event.dataTransfer.effectAllowed = "move";
            event.dataTransfer.setData("text/plain", panel);
            try {
              event.dataTransfer.setData("application/x-catalyst-panel", panel);
            } catch {
              // text/plain remains available as the cross-browser fallback.
            }
            onDragStart(panel);
          }}
          onDragEnd={onDragEnd}
          onClick={() => ide.selectDockPanel("main", panel)}
          title={`Drag ${LABELS[panel]} to another dock`}
          className={`group flex min-w-0 items-center gap-1.5 border-r border-ink-800 px-3 text-xs ${activePanel === panel ? "bg-ink-950 text-ink-100" : "text-ink-400 hover:bg-ink-900"}`}
        >
          <span className="cursor-grab select-none text-ink-600">⠿</span>
          <span className="truncate">{LABELS[panel]}</span>
          <span
            role="button"
            tabIndex={0}
            onClick={(event) => { event.stopPropagation(); ide.hideDockPanel(panel); }}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") ide.hideDockPanel(panel);
            }}
            className="ml-1 rounded p-0.5 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
            aria-label={`Close ${LABELS[panel]}`}
          >
            <XIcon width={12} height={12} />
          </span>
        </button>
      ))}
    </div>
  );
}

function MainContent({ workspace }: { workspace: string }) {
  const { ide } = useIdeContext();
  const tab = ide.state.openTabs.find((item) => item.id === ide.state.activeTabId) ?? null;
  if (!tab) return <div className="flex h-full items-center justify-center px-6 text-center text-sm text-ink-600">Open a file from the Explorer to start editing.</div>;
  if (tab.kind === "file") return <Editor tab={tab} />;
  if (tab.kind === "preview") return <Preview target={tab.target} workspace={workspace} />;
  return null;
}

function StatusBar({ connected, workspace, git }: { connected: boolean; workspace: string; git: GitStatus | null }) {
  const branch = git?.branch;
  const changes = git?.entries.length ?? 0;
  const wsName = workspace ? workspace.split(/[\\/]/).pop() ?? workspace : "—";
  return (
    <div className="flex h-6 shrink-0 items-center justify-between border-t border-ink-700 bg-ink-900 px-2 text-[11px] text-ink-300">
      <div className="flex items-center gap-2 overflow-hidden">
        {branch ? <span className="flex items-center gap-1 whitespace-nowrap"><GitBranchIcon width={12} height={12} className="text-accent" /><span>{branch}</span><span className="text-ink-500">· {changes} {changes === 1 ? "change" : "changes"}</span></span> : <span className="text-ink-500">no git</span>}
      </div>
      <div className="flex items-center gap-3"><span className={connected ? "text-emerald-400" : "text-amber-300"}>{connected ? "● connected" : "● reconnecting…"}</span><span className="max-w-[20rem] truncate text-ink-400" title={workspace}>{wsName}</span></div>
    </div>
  );
}
