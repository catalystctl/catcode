"use client";

// Sidebar — project switcher + session history + quick actions.
//   • Project picker: a dropdown listing recent workspaces; switch/add/remove.
//   • Session list: shows the session title (auto-derived or user-renamed),
//     with inline rename (double-click or pencil → input → Enter to save).
//   • Quick actions: memory / plugins / agents / settings + reset / compact /
//     stats, with a live token/turn readout.

import { useEffect, useRef, useState } from "react";
import type { ProjectEntry, SessionEntry, Stats } from "@/lib/types";
import { relativeTime, basename, formatTokens } from "@/lib/format";
import { useOutsideClose } from "@/lib/use-outside-close";
import {
  PlusIcon, HistoryIcon, TrashIcon, CompactIcon, DotIcon, XIcon,
  BrainIcon, TerminalIcon, BoltIcon, SparkIcon, FolderIcon,
  ChevronDown, CheckIcon, FolderPlusIcon, SearchIcon, PencilIcon, RefreshIcon,
  HelpIcon,
} from "./icons";

interface Props {
  open: boolean;
  onClose: () => void;
  workspace: string;
  projects: ProjectEntry[];
  switching: boolean;
  sessions: SessionEntry[];
  currentSessionFile: string | null;
  stats: Stats | null;
  onNewSession: () => void;
  onLoadSession: (path: string) => void;
  onReset: () => void;
  onCompact: () => void;
  onStats: () => void;
  onOpenPanel: (panel: string) => void;
  onSwitchWorkspace: (path: string) => void;
  onRemoveProject: (path: string) => void;
  onDeleteSession: (path: string) => void;
  onRenameSession: (name: string, title: string) => void;
  /** Optional confirm dialog (avoids window.confirm). */
  onConfirmDelete?: (title: string) => Promise<boolean>;
}

export function Sidebar(props: Props) {
  const [query, setQuery] = useState("");
  const [projectOpen, setProjectOpen] = useState(false);
  const [newProjectPath, setNewProjectPath] = useState("");
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const projectRef = useOutsideClose(() => setProjectOpen(false));
  const renameInputRef = useRef<HTMLInputElement>(null);
  const renameCancelledRef = useRef(false);

  useEffect(() => {
    if (renaming) {
      renameCancelledRef.current = false;
      renameInputRef.current?.focus();
    }
  }, [renaming]);

  const filtered = query.trim()
    ? props.sessions.filter(
        (s) =>
          (s.title ?? "").toLowerCase().includes(query.toLowerCase()) ||
          s.name.toLowerCase().includes(query.toLowerCase()),
      )
    : props.sessions;

  const currentProject = props.projects.find((p) => p.path === props.workspace);
  const projectLabel = currentProject?.name ?? basename(props.workspace) ?? "workspace";

  const startRename = (s: SessionEntry) => {
    setRenaming(s.name);
    setRenameValue(s.title ?? basename(s.name) ?? "");
  };
  const commitRename = () => {
    if (renaming) {
      const name = renaming;
      props.onRenameSession(name, renameValue);
    }
    setRenaming(null);
  };

  return (
    <>
      {/* Mobile backdrop */}
      {props.open && (
        <div
          className="fixed inset-0 z-20 bg-black/50 backdrop-blur-sm lg:hidden"
          onClick={props.onClose}
        />
      )}
      <aside
        className={`fixed left-0 top-0 z-30 flex h-full w-72 flex-col border-r border-ink-800/80 bg-ink-950/95 backdrop-blur transition-transform duration-200 lg:static lg:z-0 lg:translate-x-0 ${
          props.open ? "translate-x-0" : "-translate-x-full"
        }`}
      >
        {/* ── Project switcher ── */}
        <div className="relative border-b border-ink-800/80" ref={projectRef}>
          <button
            onClick={() => setProjectOpen((o) => !o)}
            disabled={props.switching}
            className="flex w-full items-center gap-2 px-4 py-3 text-left transition-colors hover:bg-ink-900/60 disabled:opacity-50"
          >
            <FolderIcon width={15} height={15} className="shrink-0 text-accent-soft" />
            <div className="min-w-0 flex-1">
              <div className="truncate text-[13px] font-semibold text-ink-100">{projectLabel}</div>
              <div className="truncate font-mono text-[10px] text-ink-500">
                {props.switching ? "switching…" : basename(props.workspace) || props.workspace}
              </div>
            </div>
            <ChevronDown
              width={14}
              height={14}
              className={`shrink-0 text-ink-500 transition-transform ${projectOpen ? "rotate-180" : ""}`}
            />
          </button>
          {projectOpen && (
            <div className="absolute left-0 right-0 z-40 mt-px max-h-72 overflow-auto rounded-b-xl border border-t-0 border-ink-700 bg-ink-900 p-1 shadow-2xl shadow-black/40 animate-fade-in">
              {props.projects.length === 0 && (
                <div className="px-3 py-2 text-[12px] text-ink-600">No projects yet.</div>
              )}
              {props.projects.map((p) => {
                const active = p.path === props.workspace;
                return (
                  <div
                    key={p.path}
                    className="group/proj flex items-center gap-1 rounded-lg px-2.5 py-1.5 transition-colors hover:bg-ink-800"
                  >
                    <button
                      onClick={() => {
                        if (!active) props.onSwitchWorkspace(p.path);
                        setProjectOpen(false);
                      }}
                      className="flex min-w-0 flex-1 items-center gap-2 text-left"
                    >
                      <FolderIcon width={13} height={13} className={active ? "text-accent-soft" : "text-ink-500"} />
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-[12px] font-medium text-ink-100">{p.name}</div>
                        <div className="truncate font-mono text-[10px] text-ink-500">{p.path}</div>
                      </div>
                      {active && <CheckIcon width={13} height={13} className="shrink-0 text-accent-soft" />}
                    </button>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        props.onRemoveProject(p.path);
                      }}
                      className="shrink-0 rounded p-0.5 text-ink-600 opacity-0 transition-opacity hover:bg-danger/10 hover:text-danger group/proj:opacity-100"
                      title="Remove from list"
                    >
                      <XIcon width={12} height={12} />
                    </button>
                  </div>
                );
              })}
              {/* Add project */}
              <div className="mt-1 flex items-center gap-1.5 border-t border-ink-800 pt-1.5">
                <input
                  value={newProjectPath}
                  onChange={(e) => setNewProjectPath(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && newProjectPath.trim()) {
                      props.onSwitchWorkspace(newProjectPath.trim());
                      setNewProjectPath("");
                      setProjectOpen(false);
                    }
                  }}
                  placeholder="/path/to/project"
                  className="min-w-0 flex-1 rounded-lg border border-ink-700 bg-ink-950 px-2 py-1 font-mono text-[11px] text-ink-200 placeholder:text-ink-600 focus:border-accent/40 focus:outline-none"
                />
                <button
                  onClick={() => {
                    if (newProjectPath.trim()) {
                      props.onSwitchWorkspace(newProjectPath.trim());
                      setNewProjectPath("");
                      setProjectOpen(false);
                    }
                  }}
                  className="flex shrink-0 items-center justify-center rounded-lg bg-accent px-2 py-1 text-accent"
                  title="Add & switch to project"
                >
                  <FolderPlusIcon width={13} height={13} className="text-white" />
                </button>
              </div>
            </div>
          )}
        </div>

        {/* ── Sessions header ── */}
        <div className="flex items-center justify-between px-4 py-2.5">
          <div className="flex items-center gap-2">
            <HistoryIcon width={14} height={14} className="text-accent-soft" />
            <span className="text-[12px] font-semibold text-ink-100">Sessions</span>
            <span className="text-[10px] text-ink-600">({filtered.length})</span>
          </div>
          <button
            onClick={props.onClose}
            className="rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100 lg:hidden"
          >
            <XIcon width={16} height={16} />
          </button>
        </div>

        <div className="p-2">
          <button
            onClick={props.onNewSession}
            disabled={props.switching}
            className="flex w-full items-center justify-center gap-2 rounded-lg border border-ink-700/70 bg-ink-900/70 px-3 py-2 text-[13px] font-medium text-ink-100 transition-colors hover:border-accent/50 hover:bg-ink-850 disabled:opacity-50"
          >
            <PlusIcon width={14} height={14} /> New session
          </button>
        </div>

        <div className="px-2 pb-1">
          <div className="relative">
            <SearchIcon width={12} height={12} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-ink-600" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search sessions…"
              className="w-full rounded-lg border border-ink-700/70 bg-ink-950 py-1.5 pl-7 pr-2.5 text-[12px] text-ink-200 placeholder:text-ink-600 focus:border-accent/40 focus:outline-none"
            />
          </div>
        </div>

        {/* ── Session list ── */}
        <div className="flex-1 overflow-y-auto px-2 pb-2">
          {filtered.length === 0 ? (
            <div className="px-3 py-6 text-center text-[12px] text-ink-600">
              {props.sessions.length === 0 ? "No sessions yet." : "No matches."}
            </div>
          ) : (
            <ul className="space-y-0.5">
              {filtered.map((s) => {
                const cur = props.currentSessionFile;
                const active = !!cur && (
                  cur === s.path ||
                  cur === s.name ||
                  cur.endsWith("/" + s.name) ||
                  cur.endsWith("\\" + s.name)
                );
                const displayTitle = s.title || basename(s.name) || s.name;
                const isRenaming = renaming === s.name;
                return (
                  <li key={s.name} className="group relative">
                    <button
                      onClick={() => !isRenaming && props.onLoadSession(s.path ?? s.name)}
                      className={`flex w-full items-start gap-2 rounded-lg px-2.5 py-1.5 text-left transition-colors ${
                        active ? "bg-accent/10 text-ink-100" : "text-ink-300 hover:bg-ink-850"
                      }`}
                    >
                      {active && !isRenaming && <DotIcon className="mt-1 text-accent-soft" />}
                      <div className="min-w-0 flex-1">
                        {isRenaming ? (
                          <input
                            ref={renameInputRef}
                            value={renameValue}
                            onChange={(e) => setRenameValue(e.target.value)}
                            onClick={(e) => e.stopPropagation()}
                            onKeyDown={(e) => {
                              if (e.key === "Enter") {
                                e.preventDefault();
                                commitRename();
                              } else if (e.key === "Escape") {
                                e.preventDefault();
                                renameCancelledRef.current = true;
                                setRenaming(null);
                              }
                            }}
                            onBlur={() => {
                              if (renameCancelledRef.current) {
                                renameCancelledRef.current = false;
                                return;
                              }
                              commitRename();
                            }}
                            className="w-full rounded border border-accent/40 bg-ink-950 px-1.5 py-0.5 text-[11px] text-ink-100 focus:outline-none"
                          />
                        ) : (
                          <>
                            <div className="truncate text-[12px] leading-tight text-ink-100" title={displayTitle}>
                              {displayTitle}
                            </div>
                            <div className="flex items-center gap-1.5 text-[10px] text-ink-600">
                              <span>{relativeTime((s.mtime ?? 0) * 1000)}</span>
                              {s.messages != null && (
                                <>
                                  <span>·</span>
                                  <span>{s.messages} msg</span>
                                </>
                              )}
                            </div>
                          </>
                        )}
                      </div>
                    </button>
                    {/* Rename + delete — always visible on touch; hover-reveal on pointer devices */}
                    {!isRenaming && (
                      <div className="absolute right-1.5 top-1.5 flex items-center gap-0.5 opacity-100 sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100">
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            startRename(s);
                          }}
                          className="rounded p-1 text-ink-600 hover:bg-ink-800 hover:text-ink-100"
                          title="Rename session"
                          aria-label={`Rename ${displayTitle}`}
                        >
                          <PencilIcon width={11} height={11} />
                        </button>
                        <button
                          onClick={async (e) => {
                            e.stopPropagation();
                            const ok = props.onConfirmDelete
                              ? await props.onConfirmDelete(displayTitle)
                              : window.confirm(
                                  `Delete session "${displayTitle}"? The .jsonl file will be permanently removed.`,
                                );
                            if (ok) props.onDeleteSession(s.path ?? s.name);
                          }}
                          className="rounded p-1 text-ink-600 hover:bg-danger/10 hover:text-danger"
                          title="Delete session"
                          aria-label={`Delete ${displayTitle}`}
                        >
                          <TrashIcon width={11} height={11} />
                        </button>
                      </div>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        {/* ── Footer: quick actions + stats ── */}
        <div className="border-t border-ink-800/80 p-2">
          <div className="mb-1.5 grid grid-cols-5 gap-1.5">
            <ActionBtn icon={<BrainIcon width={13} height={13} />} label="Memory" onClick={() => props.onOpenPanel("memory")} />
            <ActionBtn icon={<TerminalIcon width={13} height={13} />} label="Plugins" onClick={() => props.onOpenPanel("plugins")} />
            <ActionBtn icon={<SparkIcon width={13} height={13} />} label="Agents" onClick={() => props.onOpenPanel("subagents")} />
            <ActionBtn icon={<BoltIcon width={13} height={13} />} label="Settings" onClick={() => props.onOpenPanel("settings")} />
            <ActionBtn icon={<HelpIcon width={13} height={13} />} label="Help" onClick={() => props.onOpenPanel("help")} />
          </div>
          <div className="grid grid-cols-3 gap-1.5">
            <ActionBtn icon={<TrashIcon width={13} height={13} />} label="Reset" onClick={props.onReset} />
            <ActionBtn icon={<CompactIcon width={13} height={13} />} label="Compact" onClick={props.onCompact} />
            <ActionBtn icon={<HistoryIcon width={13} height={13} />} label="Stats" onClick={props.onStats} />
          </div>
          {props.stats && (
            <div className="mt-2 grid grid-cols-2 gap-1.5 rounded-lg border border-ink-800/70 bg-ink-925/50 p-2 font-mono text-[10px] text-ink-400">
              <Stat label="turns" value={String(props.stats.turns)} />
              <Stat label="messages" value={String(props.stats.messages)} />
              <Stat label="tokens" value={formatTokens(props.stats.tokens_total)} />
              <Stat label="cached" value={formatTokens(props.stats.cached_tokens)} />
            </div>
          )}
        </div>
      </aside>
    </>
  );
}

function ActionBtn({
  icon,
  label,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="flex flex-col items-center gap-1 rounded-lg border border-ink-800/70 bg-ink-900/50 py-2 text-[10px] text-ink-400 transition-colors hover:border-ink-700 hover:bg-ink-850 hover:text-ink-100"
    >
      {icon}
      {label}
    </button>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between">
      <span className="text-ink-600">{label}</span>
      <span className="text-ink-300">{value}</span>
    </div>
  );
}
