"use client";

// SettingsModal — runtime configuration: default model, thinking level,
// approval mode, and bash timeout. Click-outside / Escape to close. The
// preferences themselves are persisted by the parent (localStorage); this
// modal just calls the supplied callbacks.

import { useEffect, useRef, useState, type ReactNode } from "react";
import type { ModelInfo, ReadyPayload } from "@/lib/types";
import { CheckIcon, XIcon, ModelIcon, BrainIcon, ShieldIcon, BoltIcon } from "./icons";

interface Props {
  ready: ReadyPayload | null;
  models: ModelInfo[];
  selectedModel: string | null;
  thinkingLevel: string;
  approvalMode: string;
  onSelectModel: (id: string) => void;
  onSelectThinking: (level: string) => void;
  onSetApproval: (mode: "never" | "destructive" | "always") => void;
  onSetBashTimeout: (secs: number) => void;
  onClose: () => void;
}

const DEFAULT_LEVELS = ["off", "low", "medium", "high"];
const APPROVAL_MODES: Array<"never" | "destructive" | "always"> = ["never", "destructive", "always"];
const APPROVAL_HELP: Record<string, string> = {
  never: "auto-run all",
  destructive: "ask before bash/edits",
  always: "ask for everything",
};

function useOutsideClose(onClose: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const h = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const k = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", h);
    document.addEventListener("keydown", k);
    return () => {
      document.removeEventListener("mousedown", h);
      document.removeEventListener("keydown", k);
    };
  }, [onClose]);
  return ref;
}

function Section({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="mb-5">
      <div className="mb-2 text-[11px] font-medium uppercase tracking-wider text-ink-500">{label}</div>
      {children}
    </div>
  );
}

export function SettingsModal(props: Props) {
  const ref = useOutsideClose(props.onClose);
  const current = props.models.find((m) => m.id === props.selectedModel) ?? props.models[0];
  const levels = current?.thinking_levels?.length ? current.thinking_levels : DEFAULT_LEVELS;
  const effLevels = current?.reasoning ? levels : ["off"];

  const [timeoutInput, setTimeoutInput] = useState(
    String(props.ready?.bash_timeout_secs ?? 30),
  );

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div
        ref={ref}
        className="flex max-h-[85vh] w-full max-w-md flex-col overflow-hidden rounded-2xl border border-ink-700 bg-ink-900 shadow-2xl animate-fade-in"
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-ink-800/80 px-5 py-3.5">
          <div className="flex items-center gap-2">
            <BoltIcon width={16} height={16} className="text-accent-soft" />
            <h2 className="text-[15px] font-semibold text-ink-100">Settings</h2>
          </div>
          <button
            onClick={props.onClose}
            className="rounded-md p-1 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
          >
            <XIcon width={16} height={16} />
          </button>
        </div>

        <div className="overflow-y-auto px-5 py-4">
          {/* Model */}
          <Section label="Model">
            <div className="overflow-hidden rounded-xl border border-ink-800">
              {props.models.length === 0 && (
                <div className="px-3 py-2.5 text-[12px] text-ink-500">No models — set an API key.</div>
              )}
              {props.models.map((mo, i) => {
                const active = props.selectedModel === mo.id;
                return (
                  <button
                    key={mo.id}
                    onClick={() => props.onSelectModel(mo.id)}
                    className={`flex w-full items-center gap-2.5 px-3 py-2 text-left transition-colors ${
                      active ? "bg-accent/10" : "hover:bg-ink-850"
                    } ${i > 0 ? "border-t border-ink-800" : ""}`}
                  >
                    <ModelIcon width={13} height={13} className={active ? "text-accent-soft" : "text-ink-500"} />
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-[12px] font-medium text-ink-100">{mo.name || mo.id}</div>
                      <div className="truncate font-mono text-[10px] text-ink-500">{mo.id}</div>
                    </div>
                    {mo.reasoning && (
                      <span className="rounded bg-accent/15 px-1.5 py-0.5 text-[9px] font-medium uppercase tracking-wide text-accent-soft">
                        reasoning
                      </span>
                    )}
                    {active && <CheckIcon width={13} height={13} className="shrink-0 text-accent-soft" />}
                  </button>
                );
              })}
            </div>
          </Section>

          {/* Thinking */}
          <Section label="Thinking">
            <div className="flex flex-wrap gap-1.5">
              {effLevels.map((lv) => {
                const active = props.thinkingLevel === lv;
                return (
                  <button
                    key={lv}
                    onClick={() => props.onSelectThinking(lv)}
                    className={`flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-[12px] font-medium capitalize transition-colors ${
                      active
                        ? "border-accent/50 bg-accent/15 text-accent-soft"
                        : "border-ink-700/70 bg-ink-900/70 text-ink-300 hover:border-ink-600 hover:bg-ink-850"
                    }`}
                  >
                    <BrainIcon width={12} height={12} />
                    {lv}
                  </button>
                );
              })}
            </div>
          </Section>

          {/* Approval */}
          <Section label="Approval">
            <div className="space-y-1.5">
              {APPROVAL_MODES.map((mode) => {
                const active = props.approvalMode === mode;
                return (
                  <button
                    key={mode}
                    onClick={() => props.onSetApproval(mode)}
                    className={`flex w-full items-center gap-2.5 rounded-lg border px-3 py-2 text-left transition-colors ${
                      active
                        ? mode === "always"
                          ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-300"
                          : mode === "never"
                            ? "border-ink-700 bg-ink-850 text-ink-100"
                            : "border-amber-500/40 bg-amber-500/10 text-amber-300"
                        : "border-ink-700/70 bg-ink-900/70 text-ink-300 hover:border-ink-600 hover:bg-ink-850"
                    }`}
                  >
                    <ShieldIcon width={13} height={13} className="shrink-0" />
                    <div className="min-w-0 flex-1">
                      <div className="text-[12px] font-medium capitalize">{mode}</div>
                      <div className="text-[10px] text-ink-500">{APPROVAL_HELP[mode]}</div>
                    </div>
                    {active && <CheckIcon width={13} height={13} className="shrink-0" />}
                  </button>
                );
              })}
            </div>
          </Section>

          {/* Bash timeout */}
          <Section label="Bash timeout">
            <div className="flex items-center gap-2">
              <input
                type="number"
                min={1}
                value={timeoutInput}
                onChange={(e) => setTimeoutInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    const n = Number(timeoutInput);
                    if (Number.isFinite(n) && n > 0) props.onSetBashTimeout(n);
                  }
                }}
                className="w-24 rounded-lg border border-ink-700 bg-ink-950 px-3 py-1.5 font-mono text-[13px] text-ink-100 focus:border-accent/50 focus:outline-none"
              />
              <span className="text-[12px] text-ink-500">seconds</span>
              <button
                onClick={() => {
                  const n = Number(timeoutInput);
                  if (Number.isFinite(n) && n > 0) props.onSetBashTimeout(n);
                }}
                className="ml-auto rounded-lg bg-accent px-3 py-1.5 text-[12px] font-semibold text-white transition-colors hover:bg-accent-soft"
              >
                Apply
              </button>
            </div>
          </Section>
        </div>

        <div className="border-t border-ink-800/80 px-5 py-2.5">
          <p className="text-[11px] text-ink-500">Preferences are saved to this browser.</p>
        </div>
      </div>
    </div>
  );
}
