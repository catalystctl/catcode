"use client";

// SettingsModal — runtime configuration: default model, thinking level,
// approval mode, bash timeout, auto-compact, and vision handoff. Takes up 80%
// of the viewport so the model picker (with search + provider filters) has room
// to breathe. Click-outside / Escape to close. Preferences are persisted by the
// parent (localStorage); this modal just calls the supplied callbacks.

import { useEffect, useMemo, useState, type ReactNode } from "react";
import type { ModelInfo, ReadyPayload, VisionConfig } from "@/lib/types";
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
  autoCompact: boolean;
  sandbox: string;
  onSelectModel: (id: string) => void;
  onSelectThinking: (level: string) => void;
  onSetApproval: (mode: "never" | "destructive" | "always") => void;
  onSetBashTimeout: (secs: number) => void;
  onSetAutoCompact: (on: boolean) => void;
  onSetSandbox: (mode: "none" | "firejail" | "seatbelt") => void;
  visionConfig: VisionConfig | null;
  onSetVisionConfig: (vision_model: string | null, vision_models: string[]) => void;
  onRefreshVision?: () => void;
  onClose: () => void;
}

const DEFAULT_LEVELS = ["off", "low", "medium", "high"];
const APPROVAL_MODES: Array<"never" | "destructive" | "always"> = ["never", "destructive", "always"];
const APPROVAL_HELP: Record<string, string> = {
  never: "auto-run all",
  destructive: "ask before bash/edits",
  always: "ask for everything",
};
const SANDBOX_MODES: Array<"none" | "firejail" | "seatbelt"> = ["none", "firejail", "seatbelt"];
const SANDBOX_HELP: Record<string, string> = {
  none: "denylist tripwire only",
  firejail: "wrap bash in firejail (Linux)",
  seatbelt: "sandbox-exec seatbelt (macOS)",
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

  const curatedIds = useMemo(
    () => props.visionConfig?.vision_models ?? [],
    [props.visionConfig?.vision_models],
  );
  const preferredId = props.visionConfig?.vision_model ?? null;

  const [draftCurated, setDraftCurated] = useState<string[]>(() => curatedIds);
  const [draftPreferred, setDraftPreferred] = useState<string | null>(() => preferredId);

  useEffect(() => {
    setDraftCurated(curatedIds);
    setDraftPreferred(preferredId);
  }, [curatedIds, preferredId]);

  useEffect(() => {
    props.onRefreshVision?.();
    // Only refresh once when the modal mounts.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const visionCandidates = useMemo(() => {
    const byId = new Map(props.models.map((m) => [m.id, m]));
    const ids = new Set<string>();
    for (const m of props.models) {
      if (m.vision) ids.add(m.id);
    }
    for (const id of curatedIds) ids.add(id);
    for (const id of draftCurated) ids.add(id);
    return Array.from(ids)
      .map((id) => byId.get(id) ?? ({ id, name: id, vision: true } as ModelInfo))
      .sort((a, b) => (a.name || a.id).localeCompare(b.name || b.id));
  }, [props.models, curatedIds, draftCurated]);

  const toggleCurated = (id: string) => {
    setDraftCurated((prev) => {
      if (prev.includes(id)) {
        const next = prev.filter((x) => x !== id);
        if (draftPreferred === id) setDraftPreferred(next[0] ?? null);
        return next;
      }
      return [...prev, id];
    });
  };

  const saveVision = () => {
    const preferred =
      draftPreferred && draftCurated.includes(draftPreferred)
        ? draftPreferred
        : draftCurated[0] ?? null;
    props.onSetVisionConfig(preferred, draftCurated);
  };

  return (
    <div className="modal-backdrop">
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="modal-sheet max-w-5xl sm:max-h-[min(88vh,900px)]"
        role="dialog"
        aria-modal="true"
        aria-label="Settings"
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-ink-800/80 px-4 py-3.5 sm:px-6 sm:py-4">
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
        <div className="grid min-h-0 flex-1 grid-cols-1 gap-5 overflow-y-auto p-4 pb-[max(1rem,env(safe-area-inset-bottom))] sm:p-6 lg:grid-cols-2">
          {/* Left column: account + approval + timeout + auto-compact */}
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
              <ColumnLabel>Bash sandbox</ColumnLabel>
              <div className="space-y-2">
                {SANDBOX_MODES.map((mode) => {
                  const active = (props.sandbox || "none") === mode;
                  return (
                    <button
                      key={mode}
                      onClick={() => props.onSetSandbox(mode)}
                      className={`flex w-full items-center gap-2.5 rounded-xl border px-3.5 py-2.5 text-left transition-colors ${
                        active
                          ? mode === "none"
                            ? "border-ink-700 bg-ink-850 text-ink-100"
                            : "border-success/40 bg-success/10 text-success"
                          : "border-ink-700/70 bg-ink-900/70 text-ink-300 hover:border-ink-600 hover:bg-ink-850"
                      }`}
                    >
                      <ShieldIcon width={14} height={14} className="shrink-0" />
                      <div className="min-w-0 flex-1">
                        <div className="text-[13px] font-medium capitalize">{mode}</div>
                        <div className="text-[11px] text-ink-500">{SANDBOX_HELP[mode]}</div>
                      </div>
                      {active && <CheckIcon width={14} height={14} className="shrink-0" />}
                    </button>
                  );
                })}
              </div>
              <p className="mt-1.5 text-[11px] text-ink-500">
                Hard isolation for agent bash. Requires firejail (Linux) or seatbelt (macOS) when enabled.
              </p>
            </div>

            <div>
              <ColumnLabel>Auto-compact</ColumnLabel>
              <div className="flex gap-2">
                {([true, false] as const).map((on) => {
                  const active = props.autoCompact === on;
                  return (
                    <button
                      key={on ? "on" : "off"}
                      onClick={() => props.onSetAutoCompact(on)}
                      className={`flex flex-1 items-center justify-center gap-2 rounded-xl border px-3.5 py-2.5 text-[13px] font-medium transition-colors ${
                        active
                          ? on
                            ? "border-success/40 bg-success/10 text-success"
                            : "border-ink-700 bg-ink-850 text-ink-100"
                          : "border-ink-700/70 bg-ink-900/70 text-ink-300 hover:border-ink-600 hover:bg-ink-850"
                      }`}
                    >
                      {on ? "On" : "Off"}
                      {active && <CheckIcon width={14} height={14} />}
                    </button>
                  );
                })}
              </div>
              <p className="mt-1.5 text-[11px] text-ink-500">
                Automatically reclaim stale tool payloads near 70% and summarize near 90%, with
                model-specific response headroom. Off disables all automatic history rewrites;
                manual /compact always works.
              </p>
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

          {/* Right column: model picker + thinking + vision */}
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

            <div>
              <ColumnLabel>Vision handoff</ColumnLabel>
              <Panel>
                {visionCandidates.length === 0 ? (
                  <p className="text-[12px] text-ink-500">
                    No vision-capable models discovered yet.
                  </p>
                ) : (
                  <div className="space-y-1.5">
                    {visionCandidates.map((m) => {
                      const included = draftCurated.includes(m.id);
                      const preferred = draftPreferred === m.id;
                      return (
                        <div
                          key={m.id}
                          className="flex items-center gap-2 rounded-lg border border-ink-800/70 bg-ink-950/40 px-2.5 py-2"
                        >
                          <label className="flex min-w-0 flex-1 cursor-pointer items-center gap-2">
                            <input
                              type="checkbox"
                              checked={included}
                              onChange={() => toggleCurated(m.id)}
                              className="accent-[var(--accent)]"
                            />
                            <span className="truncate text-[12px] text-ink-200">
                              {m.name || m.id}
                            </span>
                            {m.vision && (
                              <span className="shrink-0 rounded bg-ink-800 px-1.5 py-0.5 text-[9px] uppercase tracking-wide text-ink-400">
                                vision
                              </span>
                            )}
                          </label>
                          <button
                            type="button"
                            title="Preferred handoff target"
                            disabled={!included}
                            onClick={() => setDraftPreferred(m.id)}
                            className={`rounded-md px-2 py-1 text-[11px] font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${
                              preferred
                                ? "bg-accent/15 text-accent-soft"
                                : "text-ink-500 hover:bg-ink-800 hover:text-ink-200"
                            }`}
                          >
                            {preferred ? "★ preferred" : "☆ prefer"}
                          </button>
                        </div>
                      );
                    })}
                  </div>
                )}
                <button
                  type="button"
                  onClick={saveVision}
                  className="mt-3 w-full rounded-lg bg-accent px-3.5 py-2 text-[12px] font-semibold text-white transition-colors hover:bg-accent-soft"
                >
                  Save vision config
                </button>
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
