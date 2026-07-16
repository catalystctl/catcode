"use client";

// SettingsModal — runtime configuration with a sidebar of focused sections.
// Preferences are persisted by the parent (localStorage / core); this modal
// just calls the supplied callbacks. Click-outside / Escape to close.

import { useEffect, useMemo, useState, type ReactNode } from "react";
import type { ModelInfo, ReadyPayload, VisionConfig } from "@/lib/types";
import { useOutsideClose, mergeRefs } from "@/lib/use-outside-close";
import { useBodyScrollLock } from "@/lib/use-body-scroll-lock";
import { useFocusTrap } from "@/lib/use-focus-trap";
import {
  CheckIcon,
  XIcon,
  BrainIcon,
  ShieldIcon,
  BoltIcon,
  ModelIcon,
  EyeIcon,
  UserIcon,
  CompactIcon,
  HelpIcon,
  LayoutIdeIcon,
  LayoutChatIcon,
} from "./icons";
import { ModelPicker } from "./model-picker";
import { AccountSecurity } from "./account-security";
import { VersionInfoPanel } from "./version-info";

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
  onSetVisionConfig: (
    vision_model: string | null,
    vision_models: string[],
    enabled?: boolean,
  ) => void;
  onRefreshVision?: () => void;
  onClose: () => void;
  /** Shell chrome mode (IDE vs chat-only). */
  uiMode?: "ide" | "chat";
  onSetUiMode?: (mode: "ide" | "chat") => void;
}

type SectionId = "appearance" | "model" | "agent" | "safety" | "vision" | "account" | "about";

const SECTIONS: Array<{
  id: SectionId;
  label: string;
  hint: string;
  icon: (p: { width?: number; height?: number; className?: string }) => ReactNode;
}> = [
  { id: "appearance", label: "Appearance", hint: "Layout & chrome", icon: LayoutIdeIcon },
  { id: "model", label: "Model", hint: "Default model & thinking", icon: ModelIcon },
  { id: "agent", label: "Agent", hint: "Timeouts & compaction", icon: CompactIcon },
  { id: "safety", label: "Safety", hint: "Approvals & sandbox", icon: ShieldIcon },
  { id: "vision", label: "Vision", hint: "Image handoff", icon: EyeIcon },
  { id: "account", label: "Account", hint: "Login & security", icon: UserIcon },
  { id: "about", label: "About", hint: "Build & updates", icon: HelpIcon },
];

const DEFAULT_LEVELS = ["off", "low", "medium", "high"];
const APPROVAL_MODES: Array<"never" | "destructive" | "always"> = ["never", "destructive", "always"];
const APPROVAL_HELP: Record<string, string> = {
  never: "Auto-run every tool without asking",
  destructive: "Ask before bash commands and file edits",
  always: "Confirm every tool call before it runs",
};
const SANDBOX_MODES: Array<"none" | "firejail" | "seatbelt"> = ["none", "firejail", "seatbelt"];
const SANDBOX_HELP: Record<string, string> = {
  none: "Denylist tripwire only — no OS sandbox",
  firejail: "Wrap bash in firejail (Linux)",
  seatbelt: "sandbox-exec seatbelt profile (macOS)",
};

function SectionHeading({ title, desc }: { title: string; desc?: string }) {
  return (
    <div className="mb-4">
      <h3 className="text-[15px] font-semibold text-ink-100">{title}</h3>
      {desc ? <p className="mt-1 text-[12px] leading-relaxed text-ink-500">{desc}</p> : null}
    </div>
  );
}

function FieldLabel({ children }: { children: ReactNode }) {
  return (
    <div className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-ink-500">
      {children}
    </div>
  );
}

function Card({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <div className={`rounded-xl border border-ink-800 bg-ink-950/40 p-4 ${className}`}>
      {children}
    </div>
  );
}

function ChoiceButton({
  active,
  tone = "accent",
  onClick,
  children,
}: {
  active: boolean;
  tone?: "accent" | "success" | "warning" | "neutral";
  onClick: () => void;
  children: ReactNode;
}) {
  const activeClass =
    tone === "success"
      ? "border-success/40 bg-success/10 text-success"
      : tone === "warning"
        ? "border-warning/40 bg-warning/10 text-warning"
        : tone === "neutral"
          ? "border-ink-600 bg-ink-850 text-ink-100"
          : "border-accent/45 bg-accent/12 text-accent-soft";
  return (
    <button
      type="button"
      onClick={onClick}
      className={`flex w-full items-center gap-3 rounded-xl border px-3.5 py-3 text-left transition-colors ${
        active
          ? activeClass
          : "border-ink-800 bg-ink-925/50 text-ink-300 hover:border-ink-700 hover:bg-ink-850"
      }`}
    >
      {children}
      {active ? <CheckIcon width={14} height={14} className="ml-auto shrink-0" /> : null}
    </button>
  );
}

function ToggleRow({
  label,
  description,
  on,
  onChange,
}: {
  label: string;
  description: string;
  on: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      onClick={() => onChange(!on)}
      className="flex w-full items-center justify-between gap-4 rounded-xl border border-ink-800 bg-ink-950/40 px-3.5 py-3 text-left transition-colors hover:border-ink-700"
    >
      <div className="min-w-0">
        <div className="text-[13px] font-medium text-ink-100">{label}</div>
        <div className="mt-0.5 text-[11px] leading-relaxed text-ink-500">{description}</div>
      </div>
      <span
        className={`relative h-5 w-9 shrink-0 rounded-full transition-colors ${
          on ? "bg-accent" : "bg-ink-700"
        }`}
      >
        <span
          className={`absolute top-0.5 h-4 w-4 rounded-full bg-white transition-transform ${
            on ? "translate-x-4" : "translate-x-0.5"
          }`}
        />
      </span>
    </button>
  );
}

export function SettingsModal(props: Props) {
  const closeRef = useOutsideClose(props.onClose);
  const trapRef = useFocusTrap<HTMLDivElement>();
  useBodyScrollLock();
  const [section, setSection] = useState<SectionId>("model");
  const current = props.models.find((m) => m.id === props.selectedModel) ?? props.models[0];
  const levels = current?.thinking_levels?.length ? current.thinking_levels : DEFAULT_LEVELS;
  const effLevels = current?.reasoning ? levels : ["off"];

  const [timeoutInput, setTimeoutInput] = useState(
    String(props.ready?.bash_timeout_secs ?? 30),
  );
  const [timeoutDirty, setTimeoutDirty] = useState(false);

  const curatedIds = useMemo(
    () => props.visionConfig?.vision_models ?? [],
    [props.visionConfig?.vision_models],
  );
  const preferredId = props.visionConfig?.vision_model ?? null;
  const enabledFromConfig = props.visionConfig?.enabled !== false;

  const [draftCurated, setDraftCurated] = useState<string[]>(() => curatedIds);
  const [draftPreferred, setDraftPreferred] = useState<string | null>(() => preferredId);
  const [draftEnabled, setDraftEnabled] = useState(enabledFromConfig);

  useEffect(() => {
    setDraftCurated(curatedIds);
    setDraftPreferred(preferredId);
    setDraftEnabled(enabledFromConfig);
  }, [curatedIds, preferredId, enabledFromConfig]);

  useEffect(() => {
    if (timeoutDirty) return;
    setTimeoutInput(String(props.ready?.bash_timeout_secs ?? 30));
  }, [props.ready?.bash_timeout_secs, timeoutDirty]);

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

  const saveVision = (enabled = draftEnabled) => {
    const preferred =
      draftPreferred && draftCurated.includes(draftPreferred)
        ? draftPreferred
        : draftCurated[0] ?? null;
    props.onSetVisionConfig(preferred, draftCurated, enabled);
  };

  const setVisionEnabled = (next: boolean) => {
    setDraftEnabled(next);
    saveVision(next);
  };

  const applyTimeout = () => {
    const n = Number(timeoutInput);
    if (Number.isFinite(n) && n > 0) {
      props.onSetBashTimeout(n);
      setTimeoutDirty(false);
    }
  };

  const activeMeta = SECTIONS.find((s) => s.id === section) ?? SECTIONS[0];

  return (
    <div className="modal-backdrop">
      <div
        ref={mergeRefs(closeRef, trapRef)}
        className="modal-sheet max-w-5xl sm:max-h-[min(88vh,860px)]"
        role="dialog"
        aria-modal="true"
        aria-label="Settings"
      >
        <div className="flex items-center justify-between border-b border-ink-800/80 px-4 py-3.5 sm:px-5">
          <div className="flex items-center gap-2.5">
            <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-accent/15 text-accent-soft">
              <BoltIcon width={16} height={16} />
            </div>
            <div>
              <h2 className="text-[15px] font-semibold text-ink-100">Settings</h2>
              <p className="text-[11px] text-ink-500">Preferences for this browser</p>
            </div>
          </div>
          <button
            onClick={props.onClose}
            className="rounded-md p-1.5 text-ink-500 hover:bg-ink-800 hover:text-ink-100"
            aria-label="Close settings"
          >
            <XIcon width={18} height={18} />
          </button>
        </div>

        <div className="flex min-h-0 flex-1 flex-col sm:flex-row">
          {/* Section nav */}
          <nav
            aria-label="Settings sections"
            className="flex shrink-0 gap-1 overflow-x-auto border-b border-ink-800/80 p-2 sm:w-52 sm:flex-col sm:overflow-y-auto sm:border-b-0 sm:border-r"
          >
            {SECTIONS.map((s) => {
              const active = section === s.id;
              const Icon = s.icon;
              return (
                <button
                  key={s.id}
                  type="button"
                  onClick={() => setSection(s.id)}
                  aria-current={active ? "page" : undefined}
                  className={`flex min-w-[7.5rem] items-center gap-2.5 rounded-lg px-2.5 py-2 text-left transition-colors sm:min-w-0 ${
                    active
                      ? "bg-accent/15 text-ink-100"
                      : "text-ink-400 hover:bg-ink-850 hover:text-ink-200"
                  }`}
                >
                  <Icon
                    width={15}
                    height={15}
                    className={active ? "shrink-0 text-accent-soft" : "shrink-0 text-ink-500"}
                  />
                  <span className="min-w-0">
                    <span className="block text-[12px] font-medium">{s.label}</span>
                    <span className="hidden truncate text-[10px] text-ink-500 sm:block">
                      {s.hint}
                    </span>
                  </span>
                </button>
              );
            })}
          </nav>

          {/* Section body */}
          <div className="min-h-0 flex-1 overflow-y-auto p-4 pb-[max(1rem,env(safe-area-inset-bottom))] sm:p-5">
            {section === "appearance" && (
              <div className="space-y-5">
                <SectionHeading
                  title="Appearance"
                  desc="Choose full IDE chrome or a chat-only layout. Your conversation stays the same."
                />
                <div>
                  <FieldLabel>Layout</FieldLabel>
                  <Card>
                    <div className="grid gap-2 sm:grid-cols-2">
                      {(
                        [
                          {
                            id: "ide" as const,
                            label: "IDE layout",
                            hint: "Explorer, editor, docks, and docked chat",
                            Icon: LayoutIdeIcon,
                          },
                          {
                            id: "chat" as const,
                            label: "Chat only",
                            hint: "Messages and composer without IDE chrome",
                            Icon: LayoutChatIcon,
                          },
                        ] as const
                      ).map(({ id, label, hint, Icon }) => {
                        const active = (props.uiMode ?? "ide") === id;
                        return (
                          <button
                            key={id}
                            type="button"
                            disabled={!props.onSetUiMode}
                            onClick={() => props.onSetUiMode?.(id)}
                            aria-pressed={active}
                            className={`flex items-start gap-3 rounded-xl border px-3 py-3 text-left transition-colors ${
                              active
                                ? "border-accent/50 bg-accent/15 text-ink-100"
                                : "border-ink-800 bg-ink-900/60 text-ink-300 hover:border-ink-700 hover:bg-ink-850"
                            } disabled:cursor-not-allowed disabled:opacity-50`}
                          >
                            <Icon width={18} height={18} className="mt-0.5 shrink-0 text-accent-soft" />
                            <span className="min-w-0">
                              <span className="block text-[13px] font-medium">{label}</span>
                              <span className="mt-0.5 block text-[11px] text-ink-500">{hint}</span>
                            </span>
                          </button>
                        );
                      })}
                    </div>
                  </Card>
                </div>
              </div>
            )}

            {section === "model" && (
              <div className="flex h-full min-h-0 flex-col gap-5">
                <SectionHeading
                  title="Model"
                  desc="Choose the default coding model and how hard it thinks."
                />
                <div className="flex min-h-[18rem] flex-1 flex-col">
                  <FieldLabel>Default model</FieldLabel>
                  <div className="min-h-0 flex-1 overflow-hidden rounded-xl border border-ink-800 bg-ink-925">
                    <ModelPicker
                      models={props.models}
                      selectedModel={props.selectedModel}
                      onSelect={props.onSelectModel}
                      variant="inline"
                    />
                  </div>
                </div>
                <div>
                  <FieldLabel>Thinking level</FieldLabel>
                  <Card>
                    <div className="flex flex-wrap gap-1.5">
                      {effLevels.map((lv) => {
                        const active = props.thinkingLevel === lv;
                        return (
                          <button
                            key={lv}
                            type="button"
                            onClick={() => props.onSelectThinking(lv)}
                            className={`flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-[12px] font-medium capitalize transition-colors ${
                              active
                                ? "border-accent/50 bg-accent/15 text-accent-soft"
                                : "border-ink-800 bg-ink-900/60 text-ink-300 hover:border-ink-700 hover:bg-ink-850"
                            }`}
                          >
                            <BrainIcon width={12} height={12} />
                            {lv}
                          </button>
                        );
                      })}
                    </div>
                    {!current?.reasoning && (
                      <p className="mt-2 text-[11px] text-ink-500">
                        This model does not expose adjustable thinking levels.
                      </p>
                    )}
                  </Card>
                </div>
              </div>
            )}

            {section === "agent" && (
              <div className="space-y-5">
                <SectionHeading
                  title="Agent"
                  desc="How long tools may run and when history is compacted."
                />
                <div>
                  <FieldLabel>Auto-compact</FieldLabel>
                  <ToggleRow
                    label="Automatically compact context"
                    description="Reclaim stale tool payloads near 70% and summarize near 90%. Manual /compact always works."
                    on={props.autoCompact}
                    onChange={props.onSetAutoCompact}
                  />
                </div>
                <div>
                  <FieldLabel>Bash timeout</FieldLabel>
                  <Card>
                    <div className="flex flex-wrap items-center gap-2">
                      <input
                        type="number"
                        min={1}
                        value={timeoutInput}
                        onChange={(e) => {
                          setTimeoutDirty(true);
                          setTimeoutInput(e.target.value);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") applyTimeout();
                        }}
                        className="w-28 rounded-lg border border-ink-700 bg-ink-950 px-3 py-2 font-mono text-[13px] text-ink-100 focus:border-accent/50 focus:outline-none"
                      />
                      <span className="text-[12px] text-ink-500">seconds</span>
                      <button
                        type="button"
                        onClick={applyTimeout}
                        className="ml-auto rounded-lg bg-accent px-3.5 py-2 text-[12px] font-semibold text-white transition-colors hover:bg-accent-soft"
                      >
                        Apply
                      </button>
                    </div>
                    <p className="mt-2 text-[11px] leading-relaxed text-ink-500">
                      Kill agent bash commands that exceed this wall-clock limit.
                    </p>
                  </Card>
                </div>
              </div>
            )}

            {section === "safety" && (
              <div className="space-y-5">
                <SectionHeading
                  title="Safety"
                  desc="Control when the agent asks for approval and how bash is isolated."
                />
                <div>
                  <FieldLabel>Approval mode</FieldLabel>
                  <div className="space-y-2">
                    {APPROVAL_MODES.map((mode) => {
                      const active = props.approvalMode === mode;
                      const tone =
                        mode === "always" ? "success" : mode === "never" ? "neutral" : "warning";
                      return (
                        <ChoiceButton
                          key={mode}
                          active={active}
                          tone={tone}
                          onClick={() => props.onSetApproval(mode)}
                        >
                          <ShieldIcon width={15} height={15} className="shrink-0" />
                          <div className="min-w-0">
                            <div className="text-[13px] font-medium capitalize">{mode}</div>
                            <div className="text-[11px] text-ink-500">{APPROVAL_HELP[mode]}</div>
                          </div>
                        </ChoiceButton>
                      );
                    })}
                  </div>
                </div>
                <div>
                  <FieldLabel>Bash sandbox</FieldLabel>
                  <div className="space-y-2">
                    {SANDBOX_MODES.map((mode) => {
                      const active = (props.sandbox || "none") === mode;
                      return (
                        <ChoiceButton
                          key={mode}
                          active={active}
                          tone={mode === "none" ? "neutral" : "success"}
                          onClick={() => props.onSetSandbox(mode)}
                        >
                          <ShieldIcon width={15} height={15} className="shrink-0" />
                          <div className="min-w-0">
                            <div className="text-[13px] font-medium capitalize">{mode}</div>
                            <div className="text-[11px] text-ink-500">{SANDBOX_HELP[mode]}</div>
                          </div>
                        </ChoiceButton>
                      );
                    })}
                  </div>
                  <p className="mt-2 text-[11px] leading-relaxed text-ink-500">
                    Hard isolation for agent bash. Requires firejail (Linux) or seatbelt (macOS)
                    when enabled.
                  </p>
                </div>
              </div>
            )}

            {section === "vision" && (
              <div className="space-y-5">
                <SectionHeading
                  title="Vision"
                  desc="When your coding model can’t see images, route that turn to a vision-capable model on the same provider."
                />
                <ToggleRow
                  label="Auto vision handoff"
                  description="Same-provider · cheapest first · recommended on"
                  on={draftEnabled}
                  onChange={setVisionEnabled}
                />
                <div>
                  <FieldLabel>Handoff models</FieldLabel>
                  <Card className={!draftEnabled ? "opacity-55" : ""}>
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
                              className="flex items-center gap-2 rounded-lg border border-ink-800/80 bg-ink-925/60 px-2.5 py-2"
                            >
                              <label
                                className={`flex min-w-0 flex-1 items-center gap-2 ${
                                  draftEnabled ? "cursor-pointer" : "cursor-not-allowed"
                                }`}
                              >
                                <input
                                  type="checkbox"
                                  checked={included}
                                  disabled={!draftEnabled}
                                  onChange={() => toggleCurated(m.id)}
                                  className="accent-accent"
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
                                disabled={!draftEnabled || !included}
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
                      onClick={() => saveVision()}
                      disabled={!draftEnabled}
                      className="mt-3 w-full rounded-lg bg-accent px-3.5 py-2 text-[12px] font-semibold text-white transition-colors hover:bg-accent-soft disabled:cursor-not-allowed disabled:bg-ink-800 disabled:text-ink-500"
                    >
                      Save vision config
                    </button>
                  </Card>
                </div>
              </div>
            )}

            {section === "account" && (
              <div>
                <SectionHeading
                  title="Account & security"
                  desc="Manage sign-in methods for this single-account instance."
                />
                <AccountSecurity />
              </div>
            )}

            {section === "about" && (
              <div>
                <SectionHeading
                  title="About"
                  desc="Build identity and whether this install is current."
                />
                <Card>
                  <VersionInfoPanel />
                </Card>
              </div>
            )}
          </div>
        </div>

        <div className="flex items-center justify-between gap-3 border-t border-ink-800/80 px-4 py-2.5 sm:px-5">
          <p className="truncate text-[11px] text-ink-500">{activeMeta.label} · {activeMeta.hint}</p>
          <button
            type="button"
            onClick={props.onClose}
            className="rounded-lg border border-ink-700 px-3 py-1.5 text-[12px] font-medium text-ink-300 hover:bg-ink-850 hover:text-ink-100"
          >
            Done
          </button>
        </div>
      </div>
    </div>
  );
}
