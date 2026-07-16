"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent } from "react";
import { flushSync } from "react-dom";
import { useAgent } from "@/lib/use-agent";
import { useIde } from "@/lib/use-ide";
import { IdeContext, useIdeContext, type AttachToChatFn } from "@/lib/ide-context";
import { useIsMobile } from "@/lib/use-media-query";
import { ChatInner } from "@/components/chat";
import type {
  DockPosition,
  FileEntry,
  GitStatus,
  MovablePanelId,
} from "@/lib/types";
import {
  FileTree,
  GitPanel,
  Editor,
  TerminalPanel,
  Preview,
  Screen,
  PANELS,
} from "./panel-registry";
import { ActivityBar } from "./activity-bar";
import { CommandPalette, type PaletteItem } from "./command-palette";
import { PanelHeader, panelTabClass } from "./panel-header";
import { ProjectSwitcher } from "./project-switcher";
import { ResizeHandle } from "./resize-handle";
import { SettingsModal } from "@/components/settings";
import {
  FileIcon,
  GitBranchIcon,
  XIcon,
  ChevronRight,
  SparkIcon,
  FolderIcon,
  TerminalIcon,
  GlobeIcon,
  BoltIcon,
} from "@/components/icons";

const MOVABLE: MovablePanelId[] = ["chat", "terminal", "git", "preview", "screen"];
const LABELS: Record<MovablePanelId, string> = {
  chat: "AI Chat",
  terminal: "Terminal",
  git: "Source Control",
  preview: "Preview",
  screen: "Screen",
};

const PANEL_MIME = "application/x-catalyst-panel";

/** Stamp drag payload and arm the drop overlay before the next dragover. */
function beginNativePanelDrag(event: DragEvent, panel: MovablePanelId) {
  event.dataTransfer.effectAllowed = "move";
  event.dataTransfer.setData("text/plain", panel);
  try {
    event.dataTransfer.setData(PANEL_MIME, panel);
  } catch {
    // Some WebKit builds only accept standard text payloads.
  }
}

function readPanelDragPayload(event: DragEvent): string {
  return event.dataTransfer.getData(PANEL_MIME) || event.dataTransfer.getData("text/plain");
}

type MobileView = "files" | "editor" | "chat" | "git" | "terminal" | "preview";

export function IdeShell() {
  const agent = useAgent();
  const workspace = agent.state.workspace;
  const ide = useIde(workspace);
  const isMobile = useIsMobile();
  const [dragging, setDragging] = useState<MovablePanelId | null>(null);
  const [mobileView, setMobileView] = useState<MobileView>("chat");
  const [projectSwitcherOpen, setProjectSwitcherOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [paletteFiles, setPaletteFiles] = useState<FileEntry[]>([]);
  const [paletteQuery, setPaletteQuery] = useState("");
  const [focusMode, setFocusMode] = useState(false);

  const chatOnly = ide.state.uiMode === "chat";

  // Chat-only clears editor zen; Esc must not exit uiMode === "chat".
  useEffect(() => {
    if (chatOnly && focusMode) setFocusMode(false);
  }, [chatOnly, focusMode]);

  // flushSync so DockDropOverlay is pointer-interactive before the browser
  // fires the next dragover — otherwise Ghostty/iframes cancel the gesture.
  const beginPanelDrag = useCallback((panel: MovablePanelId) => {
    document.body.classList.add("catalyst-panel-dragging");
    flushSync(() => setDragging(panel));
  }, []);
  const endPanelDrag = useCallback(() => {
    document.body.classList.remove("catalyst-panel-dragging");
    setDragging(null);
  }, []);

  useEffect(() => {
    return () => {
      document.body.classList.remove("catalyst-panel-dragging");
    };
  }, []);

  // Safety net: if the browser cancels DnD without dragend on the tab, clear
  // sticky overlay state when the document-level drag ends.
  useEffect(() => {
    if (!dragging) return;
    const onDocDragEnd = () => endPanelDrag();
    document.addEventListener("dragend", onDocDragEnd);
    return () => document.removeEventListener("dragend", onDocDragEnd);
  }, [dragging, endPanelDrag]);
  const openSettings = useCallback(() => {
    setProjectSwitcherOpen(false);
    setSettingsOpen(true);
    void agent.getVisionConfig();
  }, [agent]);
  const openProjects = useCallback(() => {
    setSettingsOpen(false);
    setProjectSwitcherOpen((open) => !open);
    void agent.listProjects();
  }, [agent]);
  const attachToChatRef = useRef<AttachToChatFn>(() => {});
  const attachToChat = useCallback<AttachToChatFn>((payload) => {
    attachToChatRef.current(payload);
  }, []);
  const registerAttachToChat = useCallback((fn: AttachToChatFn | null) => {
    attachToChatRef.current = fn ?? (() => {});
  }, []);
  const ctx = useMemo(
    () => ({ workspace, ide, openSettings, openProjects, attachToChat, registerAttachToChat }),
    [workspace, ide, openSettings, openProjects, attachToChat, registerAttachToChat],
  );

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      }
      if (event.key === "Escape" && focusMode && !paletteOpen) setFocusMode(false);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [focusMode, paletteOpen]);

  useEffect(() => {
    if (!paletteOpen || !workspace) {
      setPaletteFiles([]);
      return;
    }
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      void fetch(`/api/files?q=${encodeURIComponent(paletteQuery)}&workspace=${encodeURIComponent(workspace)}`, {
        signal: controller.signal,
      })
        .then((response) => response.ok ? response.json() : { files: [] })
        .then((data: { files?: FileEntry[] }) => {
          if (!controller.signal.aborted) setPaletteFiles(data.files ?? []);
        })
        .catch(() => {
          if (!controller.signal.aborted) setPaletteFiles([]);
        });
    }, 140);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [paletteOpen, paletteQuery, workspace]);

  const paletteItems = useMemo<PaletteItem[]>(() => {
    const ensureIde = () => {
      if (ide.state.uiMode !== "ide") ide.setUiMode("ide");
    };
    const panels: Array<[MovablePanelId | "explorer", string]> = [
      ["explorer", "Explorer"], ["chat", "AI Chat"], ["terminal", "Terminal"],
      ["git", "Source Control"], ["preview", "Preview"], ["screen", "Screen"],
    ];
    return [
      { id: "command:new-chat", label: "New chat", detail: "Start a fresh conversation", group: "Commands", keywords: "session", run: () => void agent.newSession() },
      {
        id: "command:ui-mode",
        label: chatOnly ? "Switch to IDE layout" : "Switch to chat-only layout",
        detail: chatOnly
          ? "Show explorer, editor, docks, and status chrome"
          : "Hide IDE chrome; keep this conversation full-bleed",
        group: "Commands",
        keywords: "layout chrome chat ide",
        run: () => ide.setUiMode(chatOnly ? "ide" : "chat"),
      },
      ...(!isMobile && !chatOnly
        ? [{
            id: "command:focus",
            label: focusMode ? "Exit focus mode" : "Focus editor",
            detail: "Distraction-free editing (keeps the editor; Esc exits)",
            group: "Commands" as const,
            keywords: "zen focus",
            run: () => setFocusMode((on) => !on),
          }]
        : []),
      { id: "command:settings", label: "Open settings", group: "Commands", run: openSettings },
      { id: "command:projects", label: "Switch project…", group: "Commands", run: openProjects },
      {
        id: "command:chat-main",
        label: "Open chat in editor area",
        detail: "Give the conversation the main workspace",
        group: "Commands",
        keywords: "expand maximize",
        run: () => {
          ensureIde();
          ide.movePanel("chat", "main");
        },
      },
      {
        id: "command:chat-right",
        label: "Dock chat on the right",
        detail: "Return chat to the side panel",
        group: "Commands",
        keywords: "restore copilot",
        run: () => {
          ensureIde();
          ide.movePanel("chat", "right");
        },
      },
      ...paletteFiles
        .filter((file) => !ide.state.openTabs.some((tab) => tab.target === file.path))
        .map((file) => ({
          id: `workspace-file:${file.path}`,
          label: file.name || file.path.split("/").pop() || file.path,
          detail: file.path,
          group: "Files" as const,
          run: () => {
            ensureIde();
            ide.openFile(file.path);
          },
        })),
      ...ide.state.openTabs.map((tab) => ({
        id: `file:${tab.id}`,
        label: tab.label,
        detail: tab.target,
        group: "Files" as const,
        run: () => {
          ensureIde();
          ide.setActiveTab(tab.id);
        },
      })),
      ...panels.map(([id, label]) => ({
        id: `panel:${id}`,
        label: `Show ${label}`,
        detail: "Open or focus panel",
        group: "Panels" as const,
        run: () => {
          ensureIde();
          if (id === "explorer") ide.selectExplorer();
          else ide.showDockPanel(id);
        },
      })),
      ...agent.state.sessions.map((session) => ({ id: `chat:${session.path ?? session.name}`, label: session.title || session.name, detail: `${session.messages ?? 0} messages`, group: "Chats" as const, run: () => void agent.loadSession(session.path ?? session.name) })),
      ...agent.state.projects.map((project) => ({ id: `project:${project.path}`, label: project.name, detail: project.path, group: "Projects" as const, run: () => void agent.switchWorkspace(project.path) })),
      ...agent.state.models.map((model) => ({ id: `model:${model.id}`, label: model.name || model.id, detail: model.provider ? `${model.provider} · ${model.id}` : model.id, group: "Models" as const, run: () => agent.setModel(model.id) })),
    ];
  }, [agent, chatOnly, focusMode, ide, isMobile, openProjects, openSettings, paletteFiles]);

  // When a file is opened from the explorer on mobile, jump to the editor.
  useEffect(() => {
    if (!isMobile) return;
    if (ide.state.activeTabId && mobileView === "files") {
      setMobileView("editor");
    }
  }, [isMobile, ide.state.activeTabId, mobileView]);

  // Prefer chat on first mobile paint if it's already visible in the layout.
  useEffect(() => {
    if (!isMobile) return;
    if (ide.state.panelVisibility.chat) setMobileView("chat");
    // Only on mobile enter.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isMobile]);

  const drop = (position: DockPosition, payload?: string) => {
    const panel = MOVABLE.includes(payload as MovablePanelId)
      ? payload as MovablePanelId
      : dragging;
    if (panel) ide.movePanel(panel, position);
    endPanelDrag();
  };

  const selectMobileView = (view: MobileView) => {
    setMobileView(view);
    if (view === "files") {
      ide.selectExplorer();
      return;
    }
    if (view === "editor") {
      ide.selectEditor();
      return;
    }
    if (view === "chat") {
      ide.showDockPanel("chat");
      return;
    }
    if (view === "terminal" && ide.state.terminals.length === 0) {
      ide.newTerminal();
    }
    ide.showDockPanel(view);
  };

  return (
    <IdeContext.Provider value={ctx}>
      <>
        {chatOnly ? (
          <div className="flex h-[100dvh] w-full overflow-hidden bg-ink-950 text-ink-100">
            <ChatInner agent={agent} docked={false} />
          </div>
        ) : isMobile ? (
          <MobileShell
            workspace={workspace}
            agent={agent}
            mobileView={mobileView}
            onSelectView={selectMobileView}
            onOpenProjects={openProjects}
            onOpenSettings={openSettings}
            connected={agent.connected}
          />
        ) : (
          <div className="relative flex h-[100dvh] w-full overflow-hidden bg-ink-950 text-ink-100">
            {!focusMode && <ActivityBar
              onOpenProjects={openProjects}
              onOpenSettings={openSettings}
              onOpenCommands={() => setPaletteOpen(true)}
            />}

          {!focusMode && !ide.state.sidebarCollapsed && (
            <>
              <PrimarySidebar
                workspace={workspace}
                agent={agent}
                onDragStart={beginPanelDrag}
                onDragEnd={endPanelDrag}
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
                  onDragStart={beginPanelDrag}
                  onDragEnd={endPanelDrag}
                />
                <EditorBreadcrumbs />
                <div className="relative min-h-0 flex-1 overflow-hidden bg-ink-950">
                  {activeMainPanel(ide) ? (
                    <PanelContent
                      panel={activeMainPanel(ide)!}
                      workspace={workspace}
                      agent={agent}
                    />
                  ) : (
                    <MainContent
                      workspace={workspace}
                      refreshToken={agent.state.fileChangeSeq}
                      changedPaths={agent.state.recentFileChanges.map((c) => c.path)}
                    />
                  )}
                </div>
              </main>

              {!focusMode && hasVisibleDock(ide, "right") && (
                <ResizeHandle
                  orientation="x"
                  invert
                  size={ide.state.copilotWidth}
                  onResize={ide.setCopilotWidth}
                  min={320}
                  max={900}
                />
              )}
              <DockAt
                position="right"
                workspace={workspace}
                agent={agent}
                onDragStart={beginPanelDrag}
                onDragEnd={endPanelDrag}
                visuallyHidden={focusMode}
              />
            </div>

            {!focusMode && hasVisibleDock(ide, "bottom") && (
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
              onDragStart={beginPanelDrag}
              onDragEnd={endPanelDrag}
              visuallyHidden={focusMode}
            />

            {!focusMode && <StatusBar
              connected={agent.connected}
              workspace={workspace}
              git={ide.state.gitStatus}
              onGit={() => ide.showDockPanel("git")}
              onWorkspace={openProjects}
              onConnection={agent.reconnect}
            />}
          </div>

            {/* Always mounted so enabling it is only a class flip (plus flushSync
                on dragstart). Conditional mount races dragover against React paint. */}
            <DockDropOverlay panel={dragging} active={dragging !== null} onDrop={drop} />
            {focusMode && (
              <button type="button" onClick={() => setFocusMode(false)} className="absolute bottom-3 left-3 z-40 rounded-md border border-ink-700 bg-ink-900/90 px-2.5 py-1.5 text-[11px] text-ink-400 shadow-lg hover:text-ink-100" title="Exit focus mode (Esc)">
                Exit focus mode
              </button>
            )}
          </div>
        )}

        {projectSwitcherOpen && (
          <ProjectSwitcher
            workspace={workspace}
            projects={agent.state.projects}
            switching={agent.state.switching}
            mobile={isMobile}
            onSwitchWorkspace={(path) => void agent.switchWorkspace(path)}
            onRemoveProject={(path) => void agent.removeProject(path)}
            onClose={() => setProjectSwitcherOpen(false)}
          />
        )}

        {settingsOpen && (
          <SettingsModal
            ready={agent.state.ready}
            models={agent.state.models}
            selectedModel={agent.state.selectedModel}
            thinkingLevel={agent.state.thinkingLevel}
            approvalMode={agent.state.approvalMode}
            autoCompact={agent.state.ready?.auto_compact ?? true}
            sandbox={agent.state.ready?.sandbox ?? "none"}
            onSelectModel={agent.setModel}
            onSelectThinking={agent.setThinking}
            onSetApproval={agent.setApproval}
            onSetBashTimeout={(secs) => void agent.setConfig("bash_timeout_secs", secs)}
            onSetAutoCompact={(on) => void agent.setConfig("auto_compact", on)}
            onSetSandbox={(mode) => void agent.setConfig("sandbox", mode)}
            visionConfig={agent.state.visionConfig}
            onSetVisionConfig={(visionModel, visionModels, enabled) =>
              void agent.setVisionConfig(visionModel, visionModels, enabled)
            }
            onRefreshVision={() => void agent.getVisionConfig()}
            onClose={() => setSettingsOpen(false)}
            uiMode={ide.state.uiMode}
            onSetUiMode={ide.setUiMode}
          />
        )}
        <CommandPalette open={paletteOpen} items={paletteItems} onClose={() => setPaletteOpen(false)} onQueryChange={setPaletteQuery} />
      </>
    </IdeContext.Provider>
  );
}

function MobileShell({
  workspace,
  agent,
  mobileView,
  onSelectView,
  onOpenProjects,
  onOpenSettings,
  connected,
}: {
  workspace: string;
  agent: ReturnType<typeof useAgent>;
  mobileView: MobileView;
  onSelectView: (view: MobileView) => void;
  onOpenProjects: () => void;
  onOpenSettings: () => void;
  connected: boolean;
}) {
  const { ide } = useIdeContext();

  return (
    <div className="relative flex h-[100dvh] w-full flex-col overflow-hidden bg-ink-950 text-ink-100 pb-[env(safe-area-inset-bottom)] pt-[env(safe-area-inset-top)]">
      <div className="flex h-10 shrink-0 items-center border-b border-ink-800 bg-ink-925 px-2">
        <button
          type="button"
          onClick={onOpenProjects}
          className="flex min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-1 text-left text-xs text-ink-200 hover:bg-ink-850"
          aria-label="Switch project"
        >
          <FolderIcon width={15} height={15} className="shrink-0 text-accent-soft" />
          <span className="truncate font-medium">
            {workspace ? workspace.split(/[\\/]/).pop() ?? workspace : "Select project"}
          </span>
          <ChevronRight width={13} height={13} className="shrink-0 rotate-90 text-ink-500" />
        </button>
        <button
          type="button"
          onClick={onOpenSettings}
          className="rounded-md p-2 text-ink-500 hover:bg-ink-850 hover:text-ink-100"
          aria-label="Settings"
          title="Settings"
        >
          <BoltIcon width={17} height={17} />
        </button>
      </div>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {(mobileView === "editor" || mobileView === "preview") && (
          <MobileTabStrip
            showPreviewTab={mobileView === "preview" || ide.state.panelVisibility.preview}
            activeView={mobileView}
            onSelectEditor={() => onSelectView("editor")}
            onSelectPreview={() => onSelectView("preview")}
          />
        )}
        <div className="relative min-h-0 flex-1 overflow-hidden bg-ink-950">
          {mobileView === "files" && <FileTree refreshToken={agent.state.fileChangeSeq} />}
          {mobileView === "editor" && (
            <MainContent
              workspace={workspace}
              onOpenPreview={() => onSelectView("preview")}
              refreshToken={agent.state.fileChangeSeq}
              changedPaths={agent.state.recentFileChanges.map((c) => c.path)}
            />
          )}
          {mobileView === "chat" && <ChatInner agent={agent} docked />}
          {mobileView === "git" && <GitPanel />}
          {mobileView === "terminal" && (
            <TerminalPanel
              workspace={workspace}
              sessions={ide.state.terminals}
              activeId={ide.state.activeTerminalId}
              onNew={() => ide.newTerminal()}
              onClose={ide.closeTerminal}
              onSelect={ide.setActiveTerminal}
              onExit={ide.setTerminalExit}
            />
          )}
          {mobileView === "preview" && (
            <Preview
              workspace={workspace}
              preview={ide.state.preview}
              onPreviewChange={ide.setPreview}
            />
          )}
        </div>
      </div>

      <StatusBar
        connected={connected}
        workspace={workspace}
        git={ide.state.gitStatus}
        compact
        onGit={() => onSelectView("git")}
        onWorkspace={onOpenProjects}
        onConnection={agent.reconnect}
      />
      <MobileBottomNav active={mobileView} onSelect={onSelectView} />
    </div>
  );
}

function MobileBottomNav({
  active,
  onSelect,
}: {
  active: MobileView;
  onSelect: (view: MobileView) => void;
}) {
  const items: Array<{ id: MobileView; label: string; icon: typeof FolderIcon }> = [
    { id: "files", label: PANELS.explorer.label, icon: FolderIcon },
    { id: "editor", label: "Editor", icon: FileIcon },
    { id: "chat", label: "Chat", icon: SparkIcon },
    { id: "git", label: "Git", icon: GitBranchIcon },
    { id: "terminal", label: "Term", icon: TerminalIcon },
    { id: "preview", label: "Preview", icon: GlobeIcon },
  ];

  return (
    <nav
      className="flex shrink-0 items-stretch border-t border-ink-800 bg-ink-925"
      aria-label="Primary"
    >
      {items.map((item) => {
        const Icon = item.icon;
        const isActive = active === item.id;
        return (
          <button
            key={item.id}
            type="button"
            onClick={() => onSelect(item.id)}
            aria-label={item.label}
            aria-current={isActive ? "page" : undefined}
            className={`flex min-h-[3.25rem] min-w-0 flex-1 flex-col items-center justify-center gap-0.5 px-0.5 text-[10px] ${
              isActive ? "text-accent-soft" : "text-ink-500"
            }`}
          >
            <Icon width={20} height={20} />
            <span className="max-w-full truncate leading-tight">{item.label}</span>
          </button>
        );
      })}
    </nav>
  );
}

function confirmCloseDirtyTab(tab: { id: string; label: string; dirty?: boolean }, closeTab: (id: string) => void) {
  if (tab.dirty && !window.confirm(`Discard unsaved changes to ${tab.label}?`)) return;
  closeTab(tab.id);
}

function MobileTabStrip({
  showPreviewTab,
  activeView,
  onSelectEditor,
  onSelectPreview,
}: {
  showPreviewTab: boolean;
  activeView: MobileView;
  onSelectEditor: () => void;
  onSelectPreview: () => void;
}) {
  const { ide } = useIdeContext();
  const { openTabs, activeTabId } = ide.state;

  return (
    <div className="flex h-9 shrink-0 items-stretch overflow-x-auto border-b border-ink-800 bg-ink-925" role="tablist">
      {openTabs.length === 0 && activeView === "editor" && (
        <span className="flex items-center px-3 text-xs text-ink-600">No open editors</span>
      )}
      {openTabs.map((tab) => {
        const active = activeView === "editor" && tab.id === activeTabId;
        return (
          <div
            key={tab.id}
            role="tab"
            tabIndex={0}
            aria-selected={active}
            onClick={() => {
              ide.setActiveTab(tab.id);
              onSelectEditor();
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                ide.setActiveTab(tab.id);
                onSelectEditor();
              }
            }}
            title={tab.target}
            className={`group flex cursor-pointer items-center gap-1.5 border-r border-ink-800 px-3 text-xs ${active ? "bg-ink-950 text-ink-100" : "text-ink-400"}`}
          >
            <FileIcon width={13} height={13} className="shrink-0 text-ink-500" />
            <span className="max-w-[10rem] truncate">{tab.label}</span>
            {tab.dirty && <span className="text-warning">●</span>}
            <button
              type="button"
              onClick={(event) => {
                event.preventDefault();
                event.stopPropagation();
                confirmCloseDirtyTab(tab, ide.closeTab);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  event.stopPropagation();
                  confirmCloseDirtyTab(tab, ide.closeTab);
                }
              }}
              className="ml-1 rounded text-ink-500 opacity-100 hover:text-ink-100 sm:opacity-0 sm:group-hover:opacity-100 sm:focus-within:opacity-100"
              aria-label={`close ${tab.label}`}
            >
              <XIcon width={12} height={12} />
            </button>
          </div>
        );
      })}
      {showPreviewTab && (
        <button
          type="button"
          onClick={onSelectPreview}
          className={`flex items-center gap-1.5 border-r border-ink-800 px-3 text-xs ${
            activeView === "preview" ? "bg-ink-950 text-ink-100" : "text-ink-400"
          }`}
        >
          <GlobeIcon width={13} height={13} className="shrink-0 text-ink-500" />
          <span>Preview</span>
        </button>
      )}
    </div>
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
      <PanelHeader trailing={(
        <button
          type="button"
          onClick={ide.toggleSidebar}
          title="Collapse sidebar"
          aria-label="Collapse sidebar"
          className="h-full px-2 text-ink-500 hover:bg-ink-850 hover:text-ink-100"
        >
          <ChevronRight width={14} height={14} />
        </button>
      )}>
        <button
          type="button"
          onClick={ide.selectExplorer}
          className={`${panelTabClass(active === null)} shrink-0 text-[11px] font-semibold uppercase tracking-wide`}
        >
          Explorer
        </button>
        {panels.map((panel) => (
          <DockPanelTab
            key={panel}
            panel={panel}
            active={active === panel}
            onSelect={() => ide.selectDockPanel("left", panel)}
            onClose={() => ide.hideDockPanel(panel)}
            onDragStart={onDragStart}
            onDragEnd={onDragEnd}
          />
        ))}
      </PanelHeader>
      <div className="min-h-0 flex-1 overflow-hidden">
        {active ? <PanelContent panel={active} workspace={workspace} agent={agent} /> : <FileTree refreshToken={agent.state.fileChangeSeq} />}
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
  visuallyHidden = false,
}: {
  position: DockPosition;
  workspace: string;
  agent: ReturnType<typeof useAgent>;
  onDragStart: (panel: MovablePanelId) => void;
  onDragEnd: () => void;
  visuallyHidden?: boolean;
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
      className={`${visuallyHidden ? "hidden" : "flex"} min-h-0 min-w-0 shrink-0 flex-col overflow-hidden border-ink-800 bg-ink-950 ${border} ${position === "main" ? "h-full w-full" : ""}`}
      aria-label={`${position} dock`}
    >
      <PanelHeader>
        {panels.map((panel) => (
          <DockPanelTab
            key={panel}
            panel={panel}
            active={panel === active}
            onSelect={() => ide.selectDockPanel(position, panel)}
            onClose={() => ide.hideDockPanel(panel)}
            onDragStart={onDragStart}
            onDragEnd={onDragEnd}
          />
        ))}
      </PanelHeader>
      <div className="min-h-0 min-w-0 flex-1 overflow-hidden">
        <PanelContent panel={active} workspace={workspace} agent={agent} />
      </div>
    </section>
  );
}

function DockPanelTab({
  panel,
  active,
  onSelect,
  onClose,
  onDragStart,
  onDragEnd,
}: {
  panel: MovablePanelId;
  active: boolean;
  onSelect: () => void;
  onClose: () => void;
  onDragStart: (panel: MovablePanelId) => void;
  onDragEnd: () => void;
}) {
  return (
    <div
      role="tab"
      tabIndex={0}
      aria-selected={active}
      draggable
      onDragStart={(event) => {
        beginNativePanelDrag(event, panel);
        onDragStart(panel);
      }}
      onDragEnd={onDragEnd}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onSelect();
        }
      }}
      title={`Drag ${LABELS[panel]} to another dock`}
      className={`${panelTabClass(active)} cursor-pointer`}
    >
      <span className="cursor-grab select-none text-ink-600 active:cursor-grabbing" aria-hidden>
        ⠿
      </span>
      <span className="truncate">{LABELS[panel]}</span>
      <button
        type="button"
        draggable={false}
        onClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          onClose();
        }}
        onMouseDown={(event) => {
          // Prevent the parent tab's HTML5 drag from starting on close clicks.
          event.stopPropagation();
        }}
        onPointerDown={(event) => {
          event.stopPropagation();
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            event.stopPropagation();
            onClose();
          }
        }}
        className="ml-auto rounded p-0.5 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
        aria-label={`Close ${LABELS[panel]}`}
      >
        <XIcon width={12} height={12} />
      </button>
    </div>
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
  if (panel === "chat") return <ChatInner agent={agent} docked />;
  if (panel === "git") return <GitPanel />;
  if (panel === "screen") return <Screen />;
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
  active,
  onDrop,
}: {
  panel: MovablePanelId | null;
  active: boolean;
  onDrop: (position: DockPosition, payload?: string) => void;
}) {
  // The full-screen layer MUST capture pointer events while active. Gaps with
  // pointer-events-none let events fall through to Ghostty's WebGL canvas
  // (and iframes), which cancels HTML5 drag mid-gesture — so the terminal
  // looked "undraggable" while chat/git still worked.
  const allowDrag = (event: DragEvent) => {
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
  };
  const label = panel ? LABELS[panel] : "panel";
  const target = (position: DockPosition, classes: string) => (
    <div
      onDragOver={allowDrag}
      onDrop={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onDrop(position, readPanelDragPayload(event));
      }}
      className={`absolute flex items-center justify-center rounded-xl border-2 border-dashed border-accent/70 bg-accent/15 text-xs font-semibold uppercase tracking-wider text-accent-soft shadow-2xl backdrop-blur-sm ${classes}`}
    >
      Dock {label} {position === "main" ? "in editor area" : position}
    </div>
  );
  return (
    <div
      aria-hidden={!active}
      className={`absolute inset-0 z-50 bg-black/20 transition-opacity ${
        active ? "pointer-events-auto opacity-100" : "pointer-events-none opacity-0"
      }`}
      onDragOver={active ? allowDrag : undefined}
      onDrop={
        active
          ? (event) => {
              // Dropping on the dimmed backdrop (not a dock target) is a no-op;
              // preventDefault so the browser doesn't navigate on text/plain.
              event.preventDefault();
            }
          : undefined
      }
    >
      {target("left", "bottom-24 left-16 top-16 w-[18%]")}
      {target("right", "bottom-24 right-4 top-16 w-[18%]")}
      {target("bottom", "bottom-8 left-[22%] right-[22%] h-[22%]")}
      {target("main", "bottom-[28%] left-[28%] right-[28%] top-[22%]")}
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
    <PanelHeader>
      {openTabs.length === 0 && panelTabs.length === 0 && <span className="flex items-center px-3 text-xs text-ink-600">No open editors</span>}
      {openTabs.map((tab) => {
        const active = activePanel === null && tab.id === activeTabId;
        return (
          <div
            key={tab.id}
            role="tab"
            tabIndex={0}
            aria-selected={active}
            onClick={() => ide.setActiveTab(tab.id)}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                ide.setActiveTab(tab.id);
              }
            }}
            title={tab.target}
            className={`${panelTabClass(active)} cursor-pointer`}
          >
            <FileIcon width={13} height={13} className="shrink-0 text-ink-500" />
            <span className="max-w-[12rem] truncate">{tab.label}</span>
            {tab.dirty && <span className="text-warning">●</span>}
            <button
              type="button"
              onClick={(event) => {
                event.preventDefault();
                event.stopPropagation();
                confirmCloseDirtyTab(tab, ide.closeTab);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  event.stopPropagation();
                  confirmCloseDirtyTab(tab, ide.closeTab);
                }
              }}
              className="ml-1 rounded text-ink-500 opacity-100 hover:text-ink-100 sm:opacity-0 sm:group-hover:opacity-100 sm:focus-within:opacity-100"
              aria-label={`close ${tab.label}`}
            >
              <XIcon width={12} height={12} />
            </button>
          </div>
        );
      })}
      {panelTabs.map((panel) => (
        <div
          key={`panel:${panel}`}
          role="tab"
          tabIndex={0}
          aria-selected={activePanel === panel}
          draggable
          onDragStart={(event) => {
            beginNativePanelDrag(event, panel);
            onDragStart(panel);
          }}
          onDragEnd={onDragEnd}
          onClick={() => ide.selectDockPanel("main", panel)}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              ide.selectDockPanel("main", panel);
            }
          }}
          title={`Drag ${LABELS[panel]} to another dock`}
          className={`${panelTabClass(activePanel === panel)} cursor-pointer`}
        >
          <span className="cursor-grab select-none text-ink-600 active:cursor-grabbing" aria-hidden>
            ⠿
          </span>
          <span className="truncate">{LABELS[panel]}</span>
          <button
            type="button"
            draggable={false}
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              ide.hideDockPanel(panel);
            }}
            onMouseDown={(event) => {
              event.stopPropagation();
            }}
            onPointerDown={(event) => {
              event.stopPropagation();
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                event.stopPropagation();
                ide.hideDockPanel(panel);
              }
            }}
            className="ml-1 rounded p-0.5 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
            aria-label={`Close ${LABELS[panel]}`}
          >
            <XIcon width={12} height={12} />
          </button>
        </div>
      ))}
    </PanelHeader>
  );
}

function MainContent({
  workspace,
  onOpenPreview,
  refreshToken,
  changedPaths,
}: {
  workspace: string;
  onOpenPreview?: () => void;
  refreshToken?: number;
  changedPaths?: string[];
}) {
  const { ide } = useIdeContext();
  const tab = ide.state.openTabs.find((item) => item.id === ide.state.activeTabId) ?? null;
  if (!tab) {
    return (
      <div className="flex h-full items-center justify-center px-6 text-center text-sm text-ink-600">
        Open a file from the Explorer to start editing.
      </div>
    );
  }
  if (tab.kind === "file") {
    return (
      <Editor
        key={tab.id}
        tab={tab}
        onOpenPreview={onOpenPreview}
        refreshToken={refreshToken}
        changedPaths={changedPaths}
      />
    );
  }
  if (tab.kind === "preview") return <Preview target={tab.target} workspace={workspace} />;
  return null;
}

function EditorBreadcrumbs() {
  const { ide } = useIdeContext();
  if (activeMainPanel(ide)) return null;
  const tab = ide.state.openTabs.find((item) => item.id === ide.state.activeTabId);
  if (!tab || tab.kind !== "file") return null;
  const parts = tab.target.split(/[\\/]/).filter(Boolean);
  return (
    <div className="flex h-7 shrink-0 items-center gap-1 overflow-x-auto border-b border-ink-850 bg-ink-950 px-3 text-[11px] text-ink-500" aria-label="File breadcrumb" title={tab.target}>
      {parts.map((part, index) => (
        <span key={`${part}:${index}`} className="flex shrink-0 items-center gap-1">
          {index > 0 ? <ChevronRight width={11} height={11} className="text-ink-700" /> : null}
          {index === parts.length - 1 ? <FileIcon width={12} height={12} className="text-accent-soft" /> : null}
          <span className={index === parts.length - 1 ? "font-medium text-ink-300" : ""}>{part}</span>
        </span>
      ))}
      {tab.dirty ? <span className="ml-1 text-warning" title="Unsaved changes">●</span> : null}
    </div>
  );
}

function StatusBar({
  connected,
  workspace,
  git,
  compact = false,
  onGit,
  onWorkspace,
  onConnection,
}: {
  connected: boolean;
  workspace: string;
  git: GitStatus | null;
  compact?: boolean;
  onGit?: () => void;
  onWorkspace?: () => void;
  onConnection?: () => void;
}) {
  const branch = git?.branch;
  const changes = git?.entries.length ?? 0;
  const wsName = workspace ? workspace.split(/[\\/]/).pop() ?? workspace : "—";
  const [versionLabel, setVersionLabel] = useState<string | null>(null);
  const [versionTitle, setVersionTitle] = useState("Version");

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const res = await fetch("/api/version", { cache: "no-store" });
        if (!res.ok) return;
        const data = (await res.json()) as {
          ok?: boolean;
          commit?: string;
          dirty?: boolean;
          statusLabel?: string;
        };
        if (cancelled || !data?.ok || !data.commit) return;
        setVersionLabel(`${data.commit}${data.dirty ? "*" : ""}`);
        setVersionTitle(data.statusLabel ? `Catalyst Code · ${data.statusLabel}` : "Catalyst Code version");
      } catch {
        /* ignore — status bar is best-effort */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="flex h-6 shrink-0 items-center justify-between gap-2 border-t border-ink-700 bg-ink-900 px-2 text-[11px] text-ink-300">
      <div className="flex min-w-0 items-center gap-2 overflow-hidden">
        {branch ? (
          <button type="button" onClick={onGit} disabled={!onGit} title="Open Source Control" className="flex min-w-0 items-center gap-1 whitespace-nowrap rounded px-1 hover:bg-ink-800 disabled:pointer-events-none">
            <GitBranchIcon width={12} height={12} className="shrink-0 text-accent" />
            <span className="truncate">{branch}</span>
            {(git?.ahead ?? 0) > 0 && <span className="text-success">↑{git?.ahead}</span>}
            {(git?.behind ?? 0) > 0 && <span className="text-warning">↓{git?.behind}</span>}
            {!compact && (
              <span className="text-ink-500">
                · {changes} {changes === 1 ? "change" : "changes"}
              </span>
            )}
            {(git?.operations?.length ?? 0) > 0 && (
              <span className="text-danger">· {git?.operations?.[0]}</span>
            )}
          </button>
        ) : (
          <span className="text-ink-500">no git</span>
        )}
      </div>
      <div className="flex min-w-0 shrink items-center gap-2 sm:gap-3">
        {versionLabel && (
          <span className="hidden font-mono text-[10px] text-ink-500 sm:inline" title={versionTitle}>
            {versionLabel}
          </span>
        )}
        <button type="button" onClick={onConnection} disabled={!onConnection} title={connected ? "Reconnect" : "Try reconnecting"} className={`${connected ? "text-success" : "text-warning"} rounded px-1 hover:bg-ink-800 disabled:pointer-events-none`}>
          {compact ? (connected ? "●" : "○") : connected ? "● connected" : "● reconnecting…"}
        </button>
        <button type="button" onClick={onWorkspace} disabled={!onWorkspace} className={`truncate rounded px-1 text-ink-400 hover:bg-ink-800 hover:text-ink-200 disabled:pointer-events-none ${compact ? "max-w-[6rem]" : "max-w-[20rem]"}`} title={onWorkspace ? `Switch project · ${workspace}` : workspace}>
          {wsName}
        </button>
      </div>
    </div>
  );
}
