"use client";

// Header — the top control bar. Model selector, thinking-level selector,
// approval-mode toggle, live metrics, and a connection indicator. Collapses
// the secondary controls on small screens.

import { useState } from "react";
import type { Metrics, UmansConc } from "@/lib/types";
import { formatTokens, formatTps, formatMs, basename } from "@/lib/format";
import { useOutsideClose } from "@/lib/use-outside-close";
import {
  ChevronDown,
  CheckIcon,
  ModelIcon,
  BrainIcon,
  ShieldIcon,
  DotIcon,
  FolderIcon,
  MenuIcon,
  RefreshIcon,
} from "./icons";

interface Props {
  connected: boolean;
  workspace: string;
  provider: string;
  models: { id: string; name: string; reasoning: boolean; thinking_levels: string[]; provider?: string }[];
  selectedModel: string | null;
  thinkingLevel: string;
  approvalMode: string;
  metrics: Metrics | null;
  /** Live Umans concurrency (used/limit); shown ahead of tps when present. */
  umansConc?: UmansConc | null;
  streaming: boolean;
  retrying: boolean;
  sessionFile: string | null;
  switching?: boolean;
  theme?: string;
  onMenuClick?: () => void;
  onSelectModel: (id: string) => void;
  onSelectThinking: (level: string) => void;
  onSetApproval: (mode: "never" | "destructive" | "always") => void;
  onReconnect?: () => void;
  onToggleTheme?: () => void;
}

const ALL_LEVELS = ["off", "low", "medium", "high"];

export function Header(props: Props) {
  const [modelOpen, setModelOpen] = useState(false);
  const [thinkOpen, setThinkOpen] = useState(false);
  const [approvOpen, setApprovOpen] = useState(false);
  const modelRef = useOutsideClose(() => setModelOpen(false));
  const thinkRef = useOutsideClose(() => setThinkOpen(false));
  const approvRef = useOutsideClose(() => setApprovOpen(false));

  const current = props.models.find((m) => m.id === props.selectedModel) ?? props.models[0];
  const levels = current?.thinking_levels?.length ? current.thinking_levels : ALL_LEVELS;
  const effLevels = current?.reasoning ? levels : ["off"];

  const m = props.metrics;
  const conc = props.umansConc;
  const approvalModes: Array<"never" | "destructive" | "always"> = ["never", "destructive", "always"];

  return (
    <header className="relative z-20 flex items-center gap-2 border-b border-ink-800/80 bg-ink-950/60 px-3 py-2 backdrop-blur sm:px-4">
      {/* Mobile sidebar toggle */}
      <button
        onClick={props.onMenuClick}
        className="rounded-md p-1.5 text-ink-400 hover:bg-ink-850 hover:text-ink-100 lg:hidden"
        aria-label="Open sessions"
      >
        <MenuIcon />
      </button>

      {/* Brand + workspace */}
      <div className="flex min-w-0 items-center gap-2">
        <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-gradient-to-br from-accent to-accent-deep text-sm font-bold text-white shadow-glow">
          u
        </span>
        <div className="hidden min-w-0 sm:block">
          <div className="flex items-center gap-1 text-[13px] font-semibold text-ink-100">
            Umans Harness
          </div>
          <div className="flex items-center gap-1 truncate text-[10px] text-ink-500">
            <FolderIcon width={10} height={10} />
            <span className="truncate font-mono">{basename(props.workspace) || props.workspace}</span>
          </div>
        </div>
      </div>

      {/* Model selector */}
      <div className="relative ml-auto" ref={modelRef}>
        <button
          onClick={() => setModelOpen((o) => !o)}
          aria-haspopup="menu"
          aria-expanded={modelOpen}
          className="flex items-center gap-1.5 rounded-lg border border-ink-700/70 bg-ink-900/70 px-2.5 py-1.5 text-[12px] font-medium text-ink-200 transition-colors hover:border-ink-600 hover:bg-ink-850"
        >
          <ModelIcon width={13} height={13} className="text-accent-soft" />
          <span className="max-w-[120px] truncate">{current?.name || current?.id || "no model"}</span>
          <ChevronDown width={12} height={12} className="text-ink-500" />
        </button>
        {modelOpen && (
          <div role="menu" className="absolute right-0 z-30 mt-1 max-h-72 w-64 overflow-auto rounded-xl border border-ink-700 bg-ink-900 p-1 shadow-2xl shadow-black/40 animate-fade-in">
            {props.models.length === 0 && (
              <div className="px-3 py-2 text-[12px] text-ink-500">No models — set an API key.</div>
            )}
            {props.models.map((mo) => (
              <button
                key={mo.id}
                role="menuitem"
                onClick={() => {
                  props.onSelectModel(mo.id);
                  setModelOpen(false);
                }}
                className="flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left transition-colors hover:bg-ink-800"
              >
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[12px] font-medium text-ink-100">{mo.name || mo.id}</div>
                  <div className="truncate font-mono text-[10px] text-ink-500">
                    {mo.id}
                    {mo.reasoning && " · reasoning"}
                  </div>
                </div>
                {props.selectedModel === mo.id && <CheckIcon width={13} height={13} className="text-accent-soft" />}
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Thinking selector */}
      <div className="relative hidden sm:block" ref={thinkRef}>
        <button
          onClick={() => setThinkOpen((o) => !o)}
          aria-haspopup="menu"
          aria-expanded={thinkOpen}
          className="flex items-center gap-1.5 rounded-lg border border-ink-700/70 bg-ink-900/70 px-2.5 py-1.5 text-[12px] font-medium text-ink-200 transition-colors hover:border-ink-600 hover:bg-ink-850"
        >
          <BrainIcon width={13} height={13} className="text-accent-soft" />
          <span className="capitalize">{props.thinkingLevel}</span>
          <ChevronDown width={12} height={12} className="text-ink-500" />
        </button>
        {thinkOpen && (
          <div role="menu" className="absolute right-0 z-30 mt-1 w-32 overflow-hidden rounded-xl border border-ink-700 bg-ink-900 p-1 shadow-2xl shadow-black/40 animate-fade-in">
            {effLevels.map((lv) => (
              <button
                key={lv}
                role="menuitem"
                onClick={() => {
                  props.onSelectThinking(lv);
                  setThinkOpen(false);
                }}
                className="flex w-full items-center justify-between rounded-lg px-2.5 py-1.5 text-left text-[12px] capitalize text-ink-200 transition-colors hover:bg-ink-800"
              >
                {lv}
                {props.thinkingLevel === lv && <CheckIcon width={12} height={12} className="text-accent-soft" />}
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Approval mode */}
      <div className="relative" ref={approvRef}>
        <button
          onClick={() => setApprovOpen((o) => !o)}
          aria-haspopup="menu"
          aria-expanded={approvOpen}
          className={`flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-[12px] font-medium transition-colors ${
            props.approvalMode === "always"
              ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-300"
              : props.approvalMode === "never"
                ? "border-ink-700/70 bg-ink-900/70 text-ink-300"
                : "border-amber-500/40 bg-amber-500/10 text-amber-300"
          }`}
          title="Approval mode"
        >
          <ShieldIcon width={13} height={13} />
          <span className="hidden capitalize md:inline">{props.approvalMode}</span>
          <ChevronDown width={12} height={12} className="text-ink-500" />
        </button>
        {approvOpen && (
          <div role="menu" className="absolute right-0 z-30 mt-1 w-40 overflow-hidden rounded-xl border border-ink-700 bg-ink-900 p-1 shadow-2xl shadow-black/40 animate-fade-in">
            {approvalModes.map((mode) => (
              <button
                key={mode}
                role="menuitem"
                onClick={() => {
                  props.onSetApproval(mode);
                  setApprovOpen(false);
                }}
                className="flex w-full items-center justify-between rounded-lg px-2.5 py-1.5 text-left text-[12px] capitalize text-ink-200 transition-colors hover:bg-ink-800"
              >
                <span>{mode}</span>
                {props.approvalMode === mode && <CheckIcon width={12} height={12} className="text-accent-soft" />}
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Metrics + connection */}
      <div className="flex items-center gap-2 pl-1">
        {m && (
          <div className="hidden items-center gap-2.5 font-mono text-[10px] text-ink-400 lg:flex">
            {m.prompt_tokens != null && <span title="input tokens">↑{formatTokens(m.prompt_tokens)}</span>}
            {m.tokens_out != null && <span title="output tokens">↓{formatTokens(m.tokens_out)}</span>}
            {conc && conc.used != null && current?.provider === conc.provider && (
              <span title="live account-wide concurrency in use / plan limit" className="text-accent-soft">
                {conc.limit == null
                  ? `Conc ${conc.used}/∞`
                  : `Conc ${conc.used}/${conc.limit}`}
              </span>
            )}
            {m.tps != null && <span title="tokens/sec" className="text-accent-soft">{formatTps(m.tps)}</span>}
            {m.ttft_ms != null && <span title="time to first token">ttft {formatMs(m.ttft_ms)}</span>}
          </div>
        )}
        {/* Theme toggle */}
        {props.onToggleTheme && (
          <button
            onClick={props.onToggleTheme}
            className="rounded-md p-1.5 text-ink-400 transition-colors hover:bg-ink-850 hover:text-ink-100"
            title={`Theme: ${props.theme ?? "dark"} (click to toggle)`}
            aria-label="Toggle theme"
          >
            {props.theme === "light" ? (
              <svg width={14} height={14} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="5" />
                <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
              </svg>
            ) : (
              <svg width={14} height={14} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
                <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
              </svg>
            )}
          </button>
        )}
        <span
          className={`flex items-center gap-1 text-[11px] ${props.connected ? "text-emerald-400" : "text-ink-500"}`}
          title={props.connected ? "connected" : "disconnected"}
        >
          <DotIcon
            className={`${props.connected ? "text-emerald-400" : "text-ink-600"} ${
              props.streaming || props.retrying || props.switching ? "animate-pulse" : ""
            }`}
          />
          <span className="hidden capitalize sm:inline">
            {props.switching
              ? "switching"
              : props.retrying
                ? "retrying"
                : props.streaming
                  ? "working"
                  : props.connected
                    ? "ready"
                    : "offline"}
          </span>
        </span>
        {/* Reconnect button when disconnected */}
        {!props.connected && props.onReconnect && !props.switching && (
          <button
            onClick={props.onReconnect}
            className="flex items-center gap-1 rounded-md border border-ink-700 px-2 py-1 text-[11px] text-ink-300 transition-colors hover:bg-ink-850 hover:text-ink-100"
            title="Reconnect to umans-core"
          >
            <RefreshIcon width={12} height={12} /> Reconnect
          </button>
        )}
      </div>
    </header>
  );
}
