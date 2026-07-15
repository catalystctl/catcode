"use client";

// Sidebar — chat session history + chat-specific quick actions.
//   • Session list: shows the session title (auto-derived or user-renamed),
//     with inline rename (double-click or pencil → input → Enter to save).
//   • Quick actions: memory / plugins / agents / help + reset / compact /
//     stats, with a live token/turn readout.

import { useEffect, useMemo, useRef, useState } from "react";
import type { SessionEntry, Stats } from "@/lib/types";
import { relativeTime, basename, formatTokens } from "@/lib/format";
import {
  readSessionPreferences,
  writeSessionPreferences,
} from "@/lib/session-preferences";
import {
  PlusIcon, HistoryIcon, TrashIcon, CompactIcon, XIcon,
  BrainIcon, TerminalIcon, SparkIcon,
  SearchIcon, PencilIcon,
  HelpIcon,
} from "./icons";

interface Props {
  /** Render as an overlay inside a constrained chat dock, regardless of viewport width. */
  embedded?: boolean;
  open: boolean;
  /** Workspace scope for per-project session organization. */
  workspace?: string;
  onClose: () => void;
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
  onDeleteSession: (path: string) => void;
  onRenameSession: (name: string, title: string) => void;
  /** Optional confirm dialog (avoids window.confirm). */
  onConfirmDelete?: (title: string) => Promise<boolean>;
  /** Session path/name currently producing output. */
  streamingSessionFile?: string | null;
}

export function Sidebar(props: Props) {
  const [query, setQuery] = useState("");
  const [pinned, setPinned] = useState<string[]>([]);
  const [archived, setArchived] = useState<string[]>([]);
  const [loadedPreferenceScope, setLoadedPreferenceScope] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const [menuAbove, setMenuAbove] = useState(false);
  const renameInputRef = useRef<HTMLInputElement>(null);
  const renameCancelledRef = useRef(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const sessionButtonRefs = useRef(new Map<string, HTMLButtonElement>());

  useEffect(() => {
    setLoadedPreferenceScope(null);
    const preferences = readSessionPreferences(props.workspace);
    setQuery(preferences.query);
    setPinned(preferences.pinned);
    setArchived(preferences.archived);
    setLoadedPreferenceScope(props.workspace ?? "__default__");
  }, [props.workspace]);

  useEffect(() => {
    if (loadedPreferenceScope !== (props.workspace ?? "__default__")) return;
    writeSessionPreferences({ query, pinned, archived }, props.workspace);
  }, [archived, pinned, loadedPreferenceScope, props.workspace, query]);

  useEffect(() => {
    if (!openMenu) return;
    requestAnimationFrame(() => menuRef.current?.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus());
    const close = (event: PointerEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) setOpenMenu(null);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpenMenu(null);
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp" && event.key !== "Home" && event.key !== "End") return;
      const items = Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []);
      if (!items.length) return;
      event.preventDefault();
      const current = items.indexOf(document.activeElement as HTMLButtonElement);
      const next = event.key === "Home" ? 0
        : event.key === "End" ? items.length - 1
          : event.key === "ArrowDown" ? (current + 1 + items.length) % items.length
            : (current - 1 + items.length) % items.length;
      items[next]?.focus();
    };
    document.addEventListener("pointerdown", close);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", close);
      document.removeEventListener("keydown", escape);
    };
  }, [openMenu]);

  useEffect(() => {
    if (renaming) {
      renameCancelledRef.current = false;
      renameInputRef.current?.focus();
    }
  }, [renaming]);

  const filtered = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    if (!normalizedQuery) return props.sessions;
    return props.sessions.filter(
      (session) =>
        (session.title ?? "").toLowerCase().includes(normalizedQuery) ||
        session.name.toLowerCase().includes(normalizedQuery),
    );
  }, [props.sessions, query]);
  const grouped = useMemo(
    () => groupSessions(filtered, new Set(pinned), new Set(archived)),
    [archived, filtered, pinned],
  );
  const visibleSessions = useMemo(() => grouped.flatMap((group) => group.sessions), [grouped]);

  const focusSession = (index: number) => {
    const session = visibleSessions[index];
    if (session) sessionButtonRefs.current.get(session.name)?.focus();
  };
  const handleSessionKeyDown = (event: React.KeyboardEvent, index: number) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp" || event.key === "Home" || event.key === "End") {
      event.preventDefault();
      const next = event.key === "ArrowDown"
        ? Math.min(index + 1, visibleSessions.length - 1)
        : event.key === "ArrowUp"
          ? Math.max(index - 1, 0)
          : event.key === "Home" ? 0 : visibleSessions.length - 1;
      focusSession(next);
    }
  };

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
          className={`${props.embedded ? "absolute" : "fixed"} inset-0 z-20 bg-black/50 backdrop-blur-sm ${props.embedded ? "" : "lg:hidden"}`}
          onClick={props.onClose}
        />
      )}
      <aside
        // Password managers such as Proton Pass may annotate form containers
        // before React hydrates. The attribute is harmless but otherwise
        // produces a false-positive hydration mismatch in development.
        suppressHydrationWarning
        className={`${props.embedded ? "absolute" : "fixed"} left-0 top-0 z-30 flex h-full w-[19rem] max-w-[88%] flex-col border-r border-ink-800/80 bg-ink-925/98 shadow-2xl shadow-black/30 backdrop-blur-xl transition-transform duration-200 ${props.embedded ? "" : "lg:static lg:z-0 lg:translate-x-0 lg:pointer-events-auto lg:shadow-none"} ${
          props.open
            ? "translate-x-0"
            : "-translate-x-full pointer-events-none lg:pointer-events-auto"
        }`}
      >
        {/* ── Sessions header ── */}
        <div className="flex items-center justify-between px-3 pb-2 pt-3">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-accent/12 text-accent-soft">
                <HistoryIcon width={14} height={14} />
              </span>
              <div>
                <h2 className="text-[13px] font-semibold leading-tight text-ink-100">Chat history</h2>
                <p className="mt-0.5 text-[10px] leading-tight text-ink-500">
                  {props.sessions.length} {props.sessions.length === 1 ? "conversation" : "conversations"}
                </p>
              </div>
            </div>
          </div>
          <button
            onClick={props.onClose}
            className={`rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100 ${props.embedded ? "" : "lg:hidden"}`}
            aria-label="Close chat history"
          >
            <XIcon width={16} height={16} />
          </button>
        </div>

        <div className="px-3 pb-2 pt-1">
          <button
            onClick={props.onNewSession}
            disabled={props.switching}
            className="flex w-full items-center justify-center gap-2 rounded-xl bg-accent px-3 py-2.5 text-[13px] font-semibold text-white shadow-sm transition-all hover:bg-accent-soft hover:shadow-glow disabled:cursor-not-allowed disabled:opacity-50"
          >
            <PlusIcon width={15} height={15} /> New chat
          </button>
        </div>

        <div className="px-3 pb-2">
          <div className="relative">
            <SearchIcon width={13} height={13} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-ink-500" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "ArrowDown" && visibleSessions.length) {
                  e.preventDefault();
                  focusSession(0);
                }
              }}
              placeholder="Search chats…"
              aria-label="Search chat history"
              className="w-full rounded-xl border border-ink-800 bg-ink-950/70 py-2 pl-8 pr-8 text-[12px] text-ink-200 outline-none transition-colors placeholder:text-ink-500 focus:border-accent/50 focus:ring-2 focus:ring-accent/10"
            />
            {query && (
              <button
                type="button"
                onClick={() => setQuery("")}
                className="absolute right-2 top-1/2 -translate-y-1/2 rounded p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-200"
                aria-label="Clear search"
              >
                <XIcon width={11} height={11} />
              </button>
            )}
          </div>
        </div>

        {/* ── Session list ── */}
        <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
          {filtered.length === 0 ? (
            <div className="mx-1 mt-3 rounded-xl border border-dashed border-ink-800 px-4 py-8 text-center">
              <HistoryIcon width={20} height={20} className="mx-auto mb-2 text-ink-600" />
              <p className="text-[12px] font-medium text-ink-300">
                {props.sessions.length === 0 ? "No conversations yet" : "No conversations found"}
              </p>
              <p className="mt-1 text-[11px] leading-relaxed text-ink-500">
                {props.sessions.length === 0
                  ? "Start a chat and it will appear here."
                  : "Try another title or clear your search."}
              </p>
            </div>
          ) : (
            grouped.map((group) => (
              <section key={group.label} className="mb-2 last:mb-0">
                <div className="sticky top-0 z-10 flex items-center justify-between bg-ink-925/95 px-2 pb-1.5 pt-2 backdrop-blur">
                  <h3 className="text-[10px] font-semibold uppercase tracking-[0.12em] text-ink-500">
                    {group.label}
                  </h3>
                  <span className="text-[10px] tabular-nums text-ink-600">{group.sessions.length}</span>
                </div>
                <ul className="space-y-1">
              {group.sessions.map((s) => {
                const visibleIndex = visibleSessions.findIndex((entry) => entry.name === s.name);
                const cur = props.currentSessionFile;
                const active = !!cur && (
                  cur === s.path ||
                  cur === s.name ||
                  cur.endsWith("/" + s.name) ||
                  cur.endsWith("\\" + s.name)
                );
                const displayTitle = s.title || basename(s.name) || s.name;
                const isRenaming = renaming === s.name;
                const isPinned = pinned.includes(s.name);
                const isArchived = archived.includes(s.name);
                const isStreaming = matchesSession(props.streamingSessionFile, s);
                return (
                  <li key={s.name} className="group relative">
                    <button
                      ref={(node) => {
                        if (node) sessionButtonRefs.current.set(s.name, node);
                        else sessionButtonRefs.current.delete(s.name);
                      }}
                      type="button"
                      disabled={props.switching}
                      onClick={() => !isRenaming && props.onLoadSession(s.path ?? s.name)}
                      onKeyDown={(event) => handleSessionKeyDown(event, visibleIndex)}
                      aria-current={active ? "page" : undefined}
                      className={`relative flex w-full items-start gap-2.5 overflow-hidden rounded-xl border px-2.5 py-2 text-left transition-all disabled:cursor-wait disabled:opacity-60 ${
                        active
                          ? "border-accent/25 bg-accent/10 text-ink-100 shadow-sm"
                          : "border-transparent text-ink-300 hover:border-ink-800 hover:bg-ink-900"
                      }`}
                    >
                      {active && <span className="absolute inset-y-2 left-0 w-0.5 rounded-r bg-accent" />}
                      <span className={`mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-lg ${active ? "bg-accent/15 text-accent-soft" : "bg-ink-850 text-ink-500"}`}>
                        <SparkIcon width={12} height={12} />
                      </span>
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
                            <div className="line-clamp-2 pr-7 text-[12px] font-medium leading-4 text-ink-100" title={displayTitle}>
                              {displayTitle}
                            </div>
                            <div className="flex items-center gap-1.5 text-[10px] leading-4 text-ink-500">
                              {isStreaming && (
                                <><span className="h-1.5 w-1.5 animate-pulse rounded-full bg-accent" aria-hidden="true" /><span className="sr-only">Generating</span></>
                              )}
                              <span>{relativeTime((s.mtime ?? 0) * 1000)}</span>
                              {s.messages != null && (
                                <>
                                  <span>·</span>
                                  <span>{s.messages} {s.messages === 1 ? "message" : "messages"}</span>
                                </>
                              )}
                            </div>
                          </>
                        )}
                      </div>
                    </button>
                    {/* Compact overflow keeps destructive actions out of the primary path. */}
                    {!isRenaming && (
                      <div ref={openMenu === s.name ? menuRef : undefined} className="absolute right-2 top-2">
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            setMenuAbove(e.currentTarget.getBoundingClientRect().bottom + 170 > window.innerHeight);
                            setOpenMenu((current) => current === s.name ? null : s.name);
                          }}
                          className="rounded-md bg-ink-925/90 p-1 text-ink-500 opacity-100 shadow-sm backdrop-blur hover:bg-ink-800 hover:text-ink-100 sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100"
                          title="Conversation actions"
                          aria-label={`Actions for ${displayTitle}`}
                          aria-haspopup="menu"
                          aria-expanded={openMenu === s.name}
                        >
                          <MoreIcon width={13} height={13} />
                        </button>
                        {openMenu === s.name && (
                          <div role="menu" aria-label={`Actions for ${displayTitle}`} className={`absolute right-0 z-30 w-36 rounded-lg border border-ink-700 bg-ink-900 p-1 shadow-xl ${menuAbove ? "bottom-7" : "top-7"}`}>
                            <SessionMenuItem icon={<PencilIcon width={12} height={12} />} label="Rename" onClick={() => { setOpenMenu(null); startRename(s); }} />
                            <SessionMenuItem icon={<PinIcon width={12} height={12} />} label={isPinned ? "Unpin" : "Pin"} onClick={() => {
                              setPinned((items) => isPinned ? items.filter((name) => name !== s.name) : [s.name, ...items]);
                              if (!isPinned) setArchived((items) => items.filter((name) => name !== s.name));
                              setOpenMenu(null);
                            }} />
                            <SessionMenuItem icon={<ArchiveIcon width={12} height={12} />} label={isArchived ? "Unarchive" : "Archive"} onClick={() => {
                              setArchived((items) => isArchived ? items.filter((name) => name !== s.name) : [s.name, ...items]);
                              if (!isArchived) setPinned((items) => items.filter((name) => name !== s.name));
                              setOpenMenu(null);
                            }} />
                            <div className="my-1 border-t border-ink-700" />
                            <SessionMenuItem danger icon={<TrashIcon width={12} height={12} />} label="Delete" onClick={async () => {
                              setOpenMenu(null);
                              const ok = props.onConfirmDelete
                                ? await props.onConfirmDelete(displayTitle)
                                : window.confirm(`Delete session "${displayTitle}"? The .jsonl file will be permanently removed.`);
                              if (ok) {
                                setPinned((items) => items.filter((name) => name !== s.name));
                                setArchived((items) => items.filter((name) => name !== s.name));
                                props.onDeleteSession(s.path ?? s.name);
                              }
                            }} />
                          </div>
                        )}
                      </div>
                    )}
                  </li>
                );
              })}
                </ul>
              </section>
            ))
          )}
        </div>

        {/* ── Footer: quick actions + stats ── */}
        <div className="border-t border-ink-800/80 bg-ink-950/45 p-2.5">
          <div className="grid grid-cols-4 gap-1">
            <ActionBtn icon={<BrainIcon width={13} height={13} />} label="Memory" onClick={() => props.onOpenPanel("memory")} />
            <ActionBtn icon={<TerminalIcon width={13} height={13} />} label="Plugins" onClick={() => props.onOpenPanel("plugins")} />
            <ActionBtn icon={<SparkIcon width={13} height={13} />} label="Agents" onClick={() => props.onOpenPanel("subagents")} />
            <ActionBtn icon={<HelpIcon width={13} height={13} />} label="Help" onClick={() => props.onOpenPanel("help")} />
          </div>
          <div className="mt-2 grid grid-cols-3 gap-1 border-t border-ink-800/70 pt-2">
            <CompactAction icon={<TrashIcon width={12} height={12} />} label="Reset" onClick={props.onReset} danger />
            <CompactAction icon={<CompactIcon width={12} height={12} />} label="Compact" onClick={props.onCompact} />
            <CompactAction icon={<HistoryIcon width={12} height={12} />} label="Usage" onClick={props.onStats} />
          </div>
          {props.stats && (
            <div className="mt-2 flex items-center justify-between rounded-lg bg-ink-900/70 px-2.5 py-1.5 font-mono text-[10px] text-ink-400">
              <span>{props.stats.turns} turns</span>
              <span className="text-ink-600">·</span>
              <span>{formatTokens(props.stats.tokens_total)} tokens</span>
              <span className="text-ink-600">·</span>
              <span>{formatTokens(props.stats.cached_tokens)} cached</span>
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
      className="flex flex-col items-center gap-1 rounded-lg py-1.5 text-[10px] text-ink-400 transition-colors hover:bg-ink-850 hover:text-ink-100"
    >
      {icon}
      {label}
    </button>
  );
}

function CompactAction({
  icon,
  label,
  onClick,
  danger,
}: {
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`flex items-center justify-center gap-1.5 rounded-lg px-1.5 py-1.5 text-[10px] transition-colors ${danger ? "text-ink-500 hover:bg-danger/10 hover:text-danger" : "text-ink-500 hover:bg-ink-850 hover:text-ink-200"}`}
    >
      {icon}
      {label}
    </button>
  );
}

function SessionMenuItem({
  icon,
  label,
  onClick,
  danger,
}: {
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      className={`flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[11px] transition-colors ${danger ? "text-danger hover:bg-danger/10" : "text-ink-300 hover:bg-ink-800 hover:text-ink-100"}`}
    >
      {icon}
      {label}
    </button>
  );
}

function MoreIcon(props: React.SVGProps<SVGSVGElement>) {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" {...props}>
      <circle cx="5" cy="12" r="2" /><circle cx="12" cy="12" r="2" /><circle cx="19" cy="12" r="2" />
    </svg>
  );
}

function PinIcon(props: React.SVGProps<SVGSVGElement>) {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" {...props}>
      <path d="m14 4 6 6-3 1-4 4-1 5-3-3-5 3 3-5 4-4 1-3 2-4Z" /><path d="m4 20 5-5" />
    </svg>
  );
}

function ArchiveIcon(props: React.SVGProps<SVGSVGElement>) {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" {...props}>
      <path d="M3 6h18v14H3z" /><path d="M2 3h20v3H2zM9 10h6" />
    </svg>
  );
}

type SessionGroup = { label: string; sessions: SessionEntry[] };

function groupSessions(sessions: SessionEntry[], pinned: Set<string>, archived: Set<string>): SessionGroup[] {
  const now = new Date();
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const day = 24 * 60 * 60 * 1000;
  const groups: SessionGroup[] = [
    { label: "Pinned", sessions: [] },
    { label: "Today", sessions: [] },
    { label: "Yesterday", sessions: [] },
    { label: "Previous 7 days", sessions: [] },
    { label: "Older", sessions: [] },
    { label: "Archived", sessions: [] },
  ];

  for (const session of sessions) {
    if (archived.has(session.name)) {
      groups[5].sessions.push(session);
      continue;
    }
    if (pinned.has(session.name)) {
      groups[0].sessions.push(session);
      continue;
    }
    const timestamp = (session.mtime ?? 0) * 1000;
    if (timestamp >= startOfToday) groups[1].sessions.push(session);
    else if (timestamp >= startOfToday - day) groups[2].sessions.push(session);
    else if (timestamp >= startOfToday - day * 7) groups[3].sessions.push(session);
    else groups[4].sessions.push(session);
  }

  return groups.filter((group) => group.sessions.length > 0);
}

function matchesSession(value: string | null | undefined, session: SessionEntry): boolean {
  return !!value && (
    value === session.path ||
    value === session.name ||
    value.endsWith("/" + session.name) ||
    value.endsWith("\\" + session.name)
  );
}
