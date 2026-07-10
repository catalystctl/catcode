"use client";

// SettingsModal — runtime configuration: default model, thinking level,
// approval mode, and bash timeout. Takes up 80% of the viewport so the model
// picker (with search + provider filters) has room to breathe. Click-outside
// / Escape to close. Preferences are persisted by the parent (localStorage);
// this modal just calls the supplied callbacks.

import { useState, type ReactNode } from "react";
import type { ModelInfo, ReadyPayload } from "@/lib/types";
import { useOutsideClose, mergeRefs } from "@/lib/use-outside-close";
import { useFocusTrap } from "@/lib/use-focus-trap";
import { CheckIcon, XIcon, BrainIcon, ShieldIcon, BoltIcon } from "./icons";
import { ModelPicker } from "./model-picker";
import { AccountSecurity } from "./account-security";

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

function ColumnLabel({ children }: { children: ReactNode }) {
  return (
    <div className="mb-2.5 text-[11px] font-semibold uppercase tracking-wider text-ink-500">
      {children}
    </div>
  );
}

function Panel({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-xl border border-ink-800 bg-ink-925/30 p-3.5">{children}</div>
  );
}

export function SettingsModal(props: Props) {
  const closeRef = useOutsideClose(props.onClose);
  const trapRef = useFocusTrap<HTMLDivElement>();
  const current = props.models.find((m) => m.id === props.selectedModel) ?? props.models[0];
  const levels = current?.thinking_levels?.length ? current.thinking_levels : DEFAULT_LEVELS;
  const effLevels = current?.reasoning ? levels : ["off"];

  const [timeoutInput, setTimeoutInput] = useState(
    String(props.ready?.bash_timeout_secs ?? 30),
  );

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="flex max-h-[88vh] w-[80vw] max-w-5xl flex-col overflow-hidden rounded-2xl border border-ink-700 bg-ink-900 shadow-2xl animate-fade-in"
        role="dialog"
        aria-modal="true"
        aria-label="Settings"
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-ink-800/80 px-6 py-4">
          <div className="flex items-center gap-2">
            <BoltIcon width={18} height={18} className="text-accent-soft" />
            <h2 className="text-[16px] font-semibold text-ink-100">Settings</h2>
          </div>
          <button
            onClick={props.onClose}
            className="rounded-md p-1.5 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
          >
            <XIcon width={18} height={18} />
          </button>
        </div>

        {/* Two-column body */}
        <div className="grid min-h-0 flex-1 grid-cols-1 gap-5 overflow-y-auto p-6 lg:grid-cols-2">
          {/* Left column: account + approval + timeout */}
          <div className="flex flex-col gap-5">
            <div>
              <ColumnLabel>Account &amp; security</ColumnLabel>
              <Panel>
                <AccountSecurity />
              </Panel>
            </div>

            <div>
              <ColumnLabel>Approval mode</ColumnLabel>
              <div className="space-y-2">
                {APPROVAL_MODES.map((mode) => {
                  const active = props.approvalMode === mode;
                  return (
                    <button
                      key={mode}
                      onClick={() => props.onSetApproval(mode)}
                      className={`flex w-full items-center gap-2.5 rounded-xl border px-3.5 py-2.5 text-left transition-colors ${
                        active
                          ? mode === "always"
                            ? "border-success/40 bg-success/10 text-success"
                            : mode === "never"
                              ? "border-ink-700 bg-ink-850 text-ink-100"
                              : "border-warning/40 bg-warning/10 text-warning"
                          : "border-ink-700/70 bg-ink-900/70 text-ink-300 hover:border-ink-600 hover:bg-ink-850"
                      }`}
                    >
                      <ShieldIcon width={14} height={14} className="shrink-0" />
                      <div className="min-w-0 flex-1">
                        <div className="text-[13px] font-medium capitalize">{mode}</div>
                        <div className="text-[11px] text-ink-500">{APPROVAL_HELP[mode]}</div>
                      </div>
                      {active && <CheckIcon width={14} height={14} className="shrink-0" />}
                    </button>
                  );
                })}
              </div>
            </div>

            <div>
              <ColumnLabel>Bash timeout</ColumnLabel>
              <Panel>
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
                    className="w-24 rounded-lg border border-ink-700 bg-ink-950 px-3 py-2 font-mono text-[13px] text-ink-100 focus:border-accent/50 focus:outline-none"
                  />
                  <span className="text-[12px] text-ink-500">seconds</span>
                  <button
                    onClick={() => {
                      const n = Number(timeoutInput);
                      if (Number.isFinite(n) && n > 0) props.onSetBashTimeout(n);
                    }}
                    className="ml-auto rounded-lg bg-accent px-3.5 py-2 text-[12px] font-semibold text-white transition-colors hover:bg-accent-soft"
                  >
                    Apply
                  </button>
                </div>
              </Panel>
            </div>
          </div>

          {/* Right column: model picker + thinking */}
          <div className="flex min-h-0 flex-col gap-5">
            <div className="flex min-h-0 flex-1 flex-col">
              <ColumnLabel>Model</ColumnLabel>
              <div className="min-h-0 flex-1 overflow-hidden rounded-xl border border-ink-800 bg-ink-900">
                <ModelPicker
                  models={props.models}
                  selectedModel={props.selectedModel}
                  onSelect={props.onSelectModel}
                  variant="inline"
                />
              </div>
            </div>

            <div>
              <ColumnLabel>Thinking level</ColumnLabel>
              <Panel>
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
              </Panel>
            </div>
          </div>
        </div>

        <div className="border-t border-ink-800/80 px-6 py-3">
          <p className="text-[11px] text-ink-500">Preferences are saved to this browser.</p>
        </div>
      </div>
    </div>
  );
}
