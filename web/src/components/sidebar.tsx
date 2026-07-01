"use client";

// Sidebar — session history + quick actions. Lists this workspace's sessions
// (most-recent first), highlights the current one, and exposes new/reset/
// compact/stats actions. Collapsible on small screens.

import { useState } from "react";
import type { SessionEntry, Stats } from "@/lib/types";
import { relativeTime, basename, formatTokens } from "@/lib/format";
import { PlusIcon, HistoryIcon, TrashIcon, CompactIcon, DotIcon, XIcon, BrainIcon, TerminalIcon, BoltIcon, SparkIcon } from "./icons";

interface Props {
  open: boolean;
  onClose: () => void;
  sessions: SessionEntry[];
  currentSessionFile: string | null;
  stats: Stats | null;
  onNewSession: () => void;
  onLoadSession: (path: string) => void;
  onReset: () => void;
  onCompact: () => void;
  onStats: () => void;
  onOpenPanel: (panel: string) => void;
}

export function Sidebar(props: Props) {
  const [query, setQuery] = useState("");
  const filtered = query.trim()
    ? props.sessions.filter((s) => s.name.toLowerCase().includes(query.toLowerCase()))
    : props.sessions;
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
        <div className="flex items-center justify-between border-b border-ink-800/80 px-4 py-3">
          <div className="flex items-center gap-2">
            <HistoryIcon width={15} height={15} className="text-accent-soft" />
            <span className="text-[13px] font-semibold text-ink-100">Sessions</span>
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
            className="flex w-full items-center justify-center gap-2 rounded-lg border border-ink-700/70 bg-ink-900/70 px-3 py-2 text-[13px] font-medium text-ink-100 transition-colors hover:border-accent/50 hover:bg-ink-850"
          >
            <PlusIcon width={14} height={14} /> New session
          </button>
        </div>

        <div className="px-2 pb-1">
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search sessions…"
            className="w-full rounded-lg border border-ink-700/70 bg-ink-950 px-2.5 py-1.5 text-[12px] text-ink-200 placeholder:text-ink-600 focus:border-accent/40 focus:outline-none"
          />
        </div>
        <div className="flex-1 overflow-y-auto px-2 pb-2">
          {filtered.length === 0 ? (
            <div className="px-3 py-6 text-center text-[12px] text-ink-600">
              {props.sessions.length === 0 ? "No sessions yet." : "No matches."}
            </div>
          ) : (
            <ul className="space-y-0.5">
              {filtered.map((s) => {
                const active = props.currentSessionFile
                  ? props.currentSessionFile.endsWith(s.name)
                  : false;
                return (
                  <li key={s.name}>
                    <button
                      onClick={() => props.onLoadSession(s.name)}
                      className={`group flex w-full items-start gap-2 rounded-lg px-2.5 py-1.5 text-left transition-colors ${
                        active ? "bg-accent/10 text-ink-100" : "text-ink-300 hover:bg-ink-850"
                      }`}
                    >
                      {active && <DotIcon className="mt-1 text-accent-soft" />}
                      <div className="min-w-0 flex-1">
                        <div className="truncate font-mono text-[11px] leading-tight">
                          {basename(s.name)}
                        </div>
                        <div className="text-[10px] text-ink-600">{relativeTime((s.mtime ?? 0) * 1000)}</div>
                      </div>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        {/* Footer: quick actions + stats */}
        <div className="border-t border-ink-800/80 p-2">
          <div className="mb-1.5 grid grid-cols-4 gap-1.5">
            <ActionBtn icon={<BrainIcon width={13} height={13} />} label="Memory" onClick={() => props.onOpenPanel("memory")} />
            <ActionBtn icon={<TerminalIcon width={13} height={13} />} label="Plugins" onClick={() => props.onOpenPanel("plugins")} />
            <ActionBtn icon={<SparkIcon width={13} height={13} />} label="Agents" onClick={() => props.onOpenPanel("subagents")} />
            <ActionBtn icon={<BoltIcon width={13} height={13} />} label="Settings" onClick={() => props.onOpenPanel("settings")} />
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
