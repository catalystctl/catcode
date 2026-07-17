"use client";

// Header — the top control bar. Model selector, thinking-level selector,
// approval-mode toggle, live metrics, and a connection indicator. Collapses
// the secondary controls on small screens.

import { useState } from "react";
import type { Metrics, UmansConc, ModelInfo, CostUpdate } from "@/lib/types";
import { formatTokens, formatTps, formatMs, basename } from "@/lib/format";
import { useOutsideClose } from "@/lib/use-outside-close";
import { ModelPicker } from "./model-picker";
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
  LayoutIdeIcon,
  BoltIcon,
} from "./icons";

interface Props {
  /** Use container-friendly chrome when hosted in an IDE dock. */
  compact?: boolean;
  connected: boolean;
  workspace: string;
  provider: string;
  models: ModelInfo[];
  selectedModel: string | null;
  thinkingLevel: string;
  approvalMode: string;
  metrics: Metrics | null;
  /** Session cost estimate from core `cost_update`. */
  cost?: CostUpdate | null;
  /** Live Umans concurrency (used/limit); shown ahead of tps when present. */
  umansConc?: UmansConc | null;
  streaming: boolean;
  retrying: boolean;
  sessionFile: string | null;
  sessionTitle?: string;
  switching?: boolean;
  theme?: string;
  onMenuClick?: () => void;
  onSelectModel: (id: string) => void;
  onSelectThinking: (level: string) => void;
  onSetApproval: (mode: "never" | "destructive" | "always") => void;
  onReconnect?: () => void;
  onToggleTheme?: () => void;
  /** When chat-only: open full IDE chrome without remounting the agent. */
  onOpenIde?: () => void;
  /** Chat-only: open Settings (activity bar is hidden in this mode). */
  onOpenSettings?: () => void;
  /** Chat-only: open project switcher (activity bar is hidden in this mode). */
  onOpenProjects?: () => void;
  /** Open Control Center mission panel. */
  onOpenControl?: () => void;
}

const ALL_LEVELS = ["off", "low", "medium", "high", "xhigh", "max"];

export function Header(props: Props) {
  const [modelOpen, setModelOpen] = useState(false);
  const [thinkOpen, setThinkOpen] = useState(false);
  const [approvOpen, setApprovOpen] = useState(false);
  const [configOpen, setConfigOpen] = useState(false);
  const modelRef = useOutsideClose(() => setModelOpen(false), modelOpen);
  const thinkRef = useOutsideClose(() => setThinkOpen(false), thinkOpen);
  const approvRef = useOutsideClose(() => setApprovOpen(false), approvOpen);
  const configRef = useOutsideClose(() => setConfigOpen(false), configOpen);

  const current = props.models.find((m) => m.id === props.selectedModel) ?? props.models[0];
  // Prefer the model's advertised thinking_levels (includes xhigh/max when supported).
  const levels =
    current?.thinking_levels && current.thinking_levels.length > 0
      ? current.thinking_levels
      : ALL_LEVELS;
  const effLevels = current?.reasoning ? levels : ["off"];

  const m = props.metrics;
  const cost = props.cost;
  const conc = props.umansConc;
  const approvalModes: Array<"never" | "destructive" | "always"> = ["never", "destructive", "always"];

  return (
    <header className={`relative z-20 flex min-w-0 items-center gap-1.5 border-b border-ink-800/60 bg-ink-950/70 px-2 backdrop-blur-md ${props.compact ? "flex-wrap py-1.5" : "py-2 sm:px-4"}`}>
      {/* Mobile sidebar toggle */}
      <button
        onClick={props.onMenuClick}
        className={`rounded-md p-1.5 text-ink-400 hover:bg-ink-850 hover:text-ink-100 ${props.compact ? "" : "lg:hidden"}`}
        aria-label="Open sessions"
      >
        <MenuIcon />
      </button>

      {/* Brand + workspace / active conversation */}
      <div className={`min-w-0 items-center gap-2 ${props.compact ? "flex flex-1" : "flex"}`}>
        {props.compact ? (
          <div className="min-w-0">
            <div className="truncate text-[12px] font-semibold leading-tight text-ink-100">
              {props.sessionTitle || "New chat"}
            </div>
            <div className="mt-0.5 flex items-center gap-1 text-[9px] leading-tight text-ink-500">
              <FolderIcon width={9} height={9} className="shrink-0" />
              <span className="truncate font-mono">{basename(props.workspace) || props.workspace}</span>
            </div>
          </div>
        ) : (
          <>
        <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-gradient-to-br from-accent to-accent-deep text-sm font-bold text-white shadow-glow">
          c
        </span>
        <div className="hidden min-w-0 sm:block">
          <div className="flex items-center gap-1 text-[13px] font-semibold text-ink-100">
            Catalyst Code
          </div>
          <div className="flex items-center gap-1 truncate text-[10px] text-ink-500">
            <FolderIcon width={10} height={10} />
            <span className="truncate font-mono">{basename(props.workspace) || props.workspace}</span>
          </div>
        </div>
          </>
        )}
      </div>

      {/* Model selector */}
      <div className={`relative ${props.compact ? "" : "ml-auto"}`} ref={modelRef}>
        <button
          onClick={() => setModelOpen((o) => !o)}
          aria-haspopup="menu"
          aria-expanded={modelOpen}
          className="flex items-center gap-1.5 rounded-lg border border-ink-700/70 bg-ink-900/70 px-2.5 py-1.5 text-[12px] font-medium text-ink-200 transition-colors hover:border-ink-600 hover:bg-ink-850"
        >
          <ModelIcon width={13} height={13} className="text-accent-soft" />
          <span className={`${props.compact ? "max-w-[82px]" : "max-w-[120px]"} truncate`}>{current?.name || current?.id || "no model"}</span>
          <ChevronDown width={12} height={12} className="text-ink-500" />
        </button>
        {modelOpen && (
          <div role="menu" className={`absolute right-0 z-30 mt-1 max-h-[min(70vh,28rem)] overflow-hidden rounded-xl border border-ink-700 bg-ink-900 p-0 shadow-2xl shadow-black/40 animate-fade-in ${props.compact ? "w-[min(18rem,calc(100vw-1rem))]" : "w-[min(20rem,calc(100vw-1rem))] sm:w-80"}`}>
            <ModelPicker
              models={props.models}
              selectedModel={props.selectedModel}
              onSelect={props.onSelectModel}
              variant="popover"
              onClose={() => setModelOpen(false)}
            />
          </div>
        )}
      </div>

      {/* Thinking selector — icon-only on xs, full on sm+ */}
      <div className={props.compact ? "hidden" : "relative"} ref={thinkRef}>
        <button
          onClick={() => setThinkOpen((o) => !o)}
          aria-haspopup="menu"
          aria-expanded={thinkOpen}
          className="flex items-center gap-1.5 rounded-lg border border-ink-700/70 bg-ink-900/70 px-2.5 py-1.5 text-[12px] font-medium text-ink-200 transition-colors hover:border-ink-600 hover:bg-ink-850"
          title={`Thinking: ${props.thinkingLevel}`}
        >
          <BrainIcon width={13} height={13} className="text-accent-soft" />
          <span className={props.compact ? "hidden" : "hidden capitalize sm:inline"}>{props.thinkingLevel}</span>
          <ChevronDown width={12} height={12} className="text-ink-500" />
        </button>
        {thinkOpen && (
          <div role="menu" className="absolute right-0 z-30 mt-1 w-40 overflow-hidden rounded-xl border border-ink-700 bg-ink-900 p-1 shadow-2xl shadow-black/40 animate-fade-in">
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
      <div className={props.compact ? "hidden" : "relative"} ref={approvRef}>
        <button
          onClick={() => setApprovOpen((o) => !o)}
          aria-haspopup="menu"
          aria-expanded={approvOpen}
          className={`flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-[12px] font-medium transition-colors ${
            props.approvalMode === "always"
              ? "border-success/40 bg-success/10 text-success"
              : props.approvalMode === "never"
                ? "border-ink-700/70 bg-ink-900/70 text-ink-300"
                : "border-warning/40 bg-warning/10 text-warning"
          }`}
          title="Approval mode"
        >
          <ShieldIcon width={13} height={13} />
          <span className={props.compact ? "hidden" : "hidden capitalize md:inline"}>{props.approvalMode}</span>
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

      {props.compact && (
        <div className="relative" ref={configRef}>
          <button
            type="button"
            onClick={() => setConfigOpen((open) => !open)}
            aria-haspopup="menu"
            aria-expanded={configOpen}
            aria-label="Chat configuration"
            title="Chat configuration"
            className="rounded-lg border border-ink-700/70 bg-ink-900/70 p-1.5 text-ink-400 transition-colors hover:border-ink-600 hover:bg-ink-850 hover:text-ink-100"
          >
            <svg width={15} height={15} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
              <path d="M4 7h10M18 7h2M4 17h2M10 17h10" />
              <circle cx="16" cy="7" r="2" /><circle cx="8" cy="17" r="2" />
            </svg>
          </button>
          {configOpen && (
            <div role="menu" className="absolute right-0 z-30 mt-1 w-56 rounded-xl border border-ink-700 bg-ink-900 p-2 shadow-2xl shadow-black/40 animate-fade-in">
              <div className="px-2 pb-1 pt-0.5 text-[10px] font-semibold uppercase tracking-wider text-ink-500">Thinking</div>
              <div className="mb-2 grid grid-cols-2 gap-1">
                {effLevels.map((level) => (
                  <button key={level} role="menuitemradio" aria-checked={props.thinkingLevel === level} onClick={() => props.onSelectThinking(level)} className={`rounded-md px-2 py-1.5 text-left text-[11px] capitalize ${props.thinkingLevel === level ? "bg-accent/15 text-accent-soft" : "text-ink-300 hover:bg-ink-800"}`}>
                    {level}
                  </button>
                ))}
              </div>
              <div className="px-2 pb-1 text-[10px] font-semibold uppercase tracking-wider text-ink-500">Approvals</div>
              <div className="grid grid-cols-3 gap-1">
                {approvalModes.map((mode) => (
                  <button key={mode} role="menuitemradio" aria-checked={props.approvalMode === mode} onClick={() => props.onSetApproval(mode)} className={`rounded-md px-1 py-1.5 text-[10px] capitalize ${props.approvalMode === mode ? "bg-accent/15 text-accent-soft" : "text-ink-300 hover:bg-ink-800"}`}>
                    {mode}
                  </button>
                ))}
              </div>
              {props.onOpenIde && (
                <button
                  role="menuitem"
                  onClick={() => {
                    props.onOpenIde?.();
                    setConfigOpen(false);
                  }}
                  className="mt-2 flex w-full items-center gap-2 border-t border-ink-800 px-2 pt-2 text-[11px] text-ink-300 hover:text-ink-100"
                >
                  <LayoutIdeIcon width={13} height={13} className="text-accent-soft" />
                  Open IDE
                </button>
              )}
              {props.onOpenProjects && (
                <button
                  role="menuitem"
                  onClick={() => {
                    props.onOpenProjects?.();
                    setConfigOpen(false);
                  }}
                  className="mt-2 flex w-full items-center gap-2 border-t border-ink-800 px-2 pt-2 text-[11px] text-ink-300 hover:text-ink-100"
                >
                  <FolderIcon width={13} height={13} className="text-accent-soft" />
                  Switch project
                </button>
              )}
              {props.onOpenControl && (
          <button
            type="button"
            onClick={props.onOpenControl}
            className="rounded-md p-1.5 text-ink-400 hover:bg-ink-850 hover:text-ink-100"
            title="Control Center"
            aria-label="Control Center"
          >
            <BoltIcon width={16} height={16} />
          </button>
        )}
        {props.onOpenSettings && (
                <button
                  role="menuitem"
                  onClick={() => {
                    props.onOpenSettings?.();
                    setConfigOpen(false);
                  }}
                  className="mt-2 flex w-full items-center gap-2 border-t border-ink-800 px-2 pt-2 text-[11px] text-ink-300 hover:text-ink-100"
                >
                  <BoltIcon width={13} height={13} className="text-accent-soft" />
                  Settings
                </button>
              )}
              {props.onToggleTheme && (
                <button role="menuitem" onClick={props.onToggleTheme} className="mt-2 flex w-full items-center justify-between border-t border-ink-800 px-2 pt-2 text-[11px] text-ink-300 hover:text-ink-100">
                  <span>Appearance</span><span className="capitalize text-ink-500">{props.theme ?? "dark"}</span>
                </button>
              )}
            </div>
          )}
        </div>
      )}

      {/* Metrics + connection */}
      <div className="flex items-center gap-2 pl-1">
        {(m || cost) && (
          <div className={`${props.compact ? "hidden" : "hidden lg:flex"} items-center gap-2.5 font-mono text-[10px] text-ink-400`}>
            {m?.prompt_tokens != null && <span title="input tokens">↑{formatTokens(m.prompt_tokens)}</span>}
            {m?.tokens_out != null && <span title="output tokens">↓{formatTokens(m.tokens_out)}</span>}
            {conc && conc.used != null && current?.provider === conc.provider && (
              <span title="live account-wide concurrency in use / plan limit" className="text-accent-soft">
                {conc.limit == null
                  ? `Conc ${conc.used}/∞`
                  : `Conc ${conc.used}/${conc.limit}`}
              </span>
            )}
            {m?.tps != null && <span title="tokens/sec" className="text-accent-soft">{formatTps(m.tps)}</span>}
            {m?.ttft_ms != null && <span title="time to first token">ttft {formatMs(m.ttft_ms)}</span>}
            {cost?.estimated_usd != null && (
              <span title="estimated session cost" className="text-ink-300">
                ${cost.estimated_usd.toFixed(cost.estimated_usd < 0.01 ? 4 : 3)}
              </span>
            )}
          </div>
        )}
        {props.onOpenIde && (
          <button
            type="button"
            onClick={props.onOpenIde}
            className="flex items-center gap-1.5 rounded-md border border-ink-700/70 bg-ink-900/70 px-2 py-1.5 text-[11px] font-medium text-ink-200 transition-colors hover:border-ink-600 hover:bg-ink-850 hover:text-ink-100"
            title="Open IDE layout"
            aria-label="Open IDE layout"
          >
            <LayoutIdeIcon width={13} height={13} className="text-accent-soft" />
            <span className="hidden sm:inline">Open IDE</span>
          </button>
        )}
        {props.onOpenProjects && (
          <button
            type="button"
            onClick={props.onOpenProjects}
            className="rounded-md p-1.5 text-ink-400 transition-colors hover:bg-ink-850 hover:text-ink-100"
            title="Switch project"
            aria-label="Switch project"
          >
            <FolderIcon width={14} height={14} />
          </button>
        )}
        {props.onOpenSettings && (
          <button
            type="button"
            onClick={props.onOpenSettings}
            className="rounded-md p-1.5 text-ink-400 transition-colors hover:bg-ink-850 hover:text-ink-100"
            title="Settings"
            aria-label="Settings"
          >
            <BoltIcon width={14} height={14} />
          </button>
        )}
        {/* Theme toggle */}
        {props.onToggleTheme && !props.compact && (
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
          className={`flex items-center gap-1 text-[11px] ${props.connected ? "text-success" : "text-ink-500"}`}
          title={props.connected ? "connected" : "disconnected"}
        >
          <DotIcon
            className={`${props.connected ? "text-success" : "text-ink-600"} ${
              props.streaming || props.retrying || props.switching ? "animate-pulse" : ""
            }`}
          />
          <span className={props.compact ? "hidden" : "hidden capitalize sm:inline"}>
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
        {!props.connected && props.onReconnect && (
          <button
            onClick={props.onReconnect}
            className="flex items-center gap-1 rounded-md border border-ink-700 px-2 py-1 text-[11px] text-ink-300 transition-colors hover:bg-ink-850 hover:text-ink-100"
            title="Reconnect to catcode-core"
          >
            <RefreshIcon width={12} height={12} /> Reconnect
          </button>
        )}
      </div>
    </header>
  );
}
