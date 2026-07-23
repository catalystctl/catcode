"use client";

// SettingsModal — runtime configuration with a sidebar of focused sections.
// Preferences are persisted by the parent (localStorage / core); this modal
// just calls the supplied callbacks. Click-outside / Escape to close.

import { useEffect, useMemo, useState, type ReactNode } from "react";
import type { ModelInfo, ReadyPayload, VisionConfig } from "@/lib/types";
import { useOutsideClose, mergeRefs } from "@/lib/use-outside-close";
import { useBodyScrollLock } from "@/lib/use-body-scroll-lock";
import { useFocusTrap } from "@/lib/use-focus-trap";
import type {
  SandboxCheckStatus,
  SandboxRuntimeStatus,
  SandboxSetupAction,
} from "@/lib/types";
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
  CopyIcon,
  RefreshIcon,
  DownloadIcon,
  WarningIcon,
  TerminalIcon,
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
  onSetSandbox: (mode: "none" | "microsandbox") => void;
  /** Live sandbox runtime status (source of truth for readiness/setup). */
  sandboxStatus: SandboxRuntimeStatus;
  /** Re-run preflight (sends get_sandbox_status). */
  onRecheckSandbox: () => void;
  /** Begin user-space runtime/image preparation (sends prepare_sandbox). */
  onPrepareSandbox: () => void;
  /** Reset an unhealthy sandbox (sends reset_sandbox). */
  onResetSandbox: () => void;
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
const SANDBOX_MODES: Array<"none" | "microsandbox"> = ["none", "microsandbox"];
const SANDBOX_HELP: Record<string, string> = {
  none: "Disabled — agent bash runs on the host (denylist tripwire only)",
  microsandbox: "Run agent bash, git, diagnostics & plugins in a Linux microVM",
};
const SANDBOX_LABEL: Record<string, string> = {
  none: "Disabled",
  microsandbox: "Microsandbox",
};
const CHECK_STATUS_META: Record<
  SandboxCheckStatus,
  { label: string; className: string; dot: string }
> = {
  pass: { label: "Pass", className: "text-success", dot: "bg-success" },
  fail: { label: "Fail", className: "text-danger", dot: "bg-danger" },
  warn: { label: "Warning", className: "text-warning", dot: "bg-warning" },
  info: { label: "Info", className: "text-ink-400", dot: "bg-ink-500" },
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

/** One row of a labeled key/value status fact (platform, image, …). */
function StatusFact({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: ReactNode;
  mono?: boolean;
}) {
  return (
    <div className="flex min-w-0 items-baseline justify-between gap-3">
      <span className="shrink-0 text-[11px] uppercase tracking-wide text-ink-500">{label}</span>
      <span
        className={`min-w-0 truncate text-right text-[12px] text-ink-200 ${mono ? "font-mono" : ""}`}
        title={typeof value === "string" ? value : undefined}
      >
        {value}
      </span>
    </div>
  );
}

/** Copy-to-clipboard button for a setup command. */
function CopyCommandButton({ command }: { command: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      onClick={() => {
        navigator.clipboard?.writeText(command).then(
          () => {
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1400);
          },
          () => {},
        );
      }}
      title="Copy command"
      className="flex shrink-0 items-center gap-1 rounded-md border border-ink-700 px-2 py-1 text-[11px] text-ink-300 transition-colors hover:bg-ink-800 hover:text-ink-100"
    >
      {copied ? (
        <CheckIcon width={12} height={12} className="text-success" />
      ) : (
        <CopyIcon width={12} height={12} />
      )}
      {copied ? "Copied" : "Copy"}
    </button>
  );
}

/** Structured Microsandbox status + setup-guidance panel. Mirrors the TUI:
 *  current status, platform, readiness, active image, CPU/memory, network mode,
 *  effective shell, preflight checks, copyable setup actions, recheck/prepare/
 *  reset/disable actions, and a core-restart hint. The core is the source of
 *  truth — no OS detection is duplicated here (platform/arch come from the
 *  report, shell from the ready payload). */
function SandboxStatusPanel({
  status,
  onRecheck,
  onPrepare,
  onReset,
  onDisable,
}: {
  status: SandboxRuntimeStatus;
  onRecheck: () => void;
  onPrepare: () => void;
  onReset: () => void;
  onDisable: () => void;
}) {
  const report = status.report;
  const requestedMicrosandbox = status.mode === "microsandbox";
  const notReady = requestedMicrosandbox && !status.ready;

  // Effective shell hint: when the microsandbox is on, the guest is always
  // Linux bash ("posix") — even on Windows — so the agent must not be told to
  // generate PowerShell. Reflect that explicitly here.
  const shellHint =
    status.mode === "microsandbox"
      ? status.shell === "powershell"
        ? "Linux bash (guest)"
        : "Linux bash (posix guest)"
      : status.shell === "powershell"
        ? "Host PowerShell"
        : "Host bash (posix)";

  return (
    <Card className="space-y-3">
      {/* Status banner */}
      <div className="flex items-center gap-2">
        <span
          className={`h-2 w-2 shrink-0 rounded-full ${
            requestedMicrosandbox
              ? status.ready
                ? "bg-success"
                : "bg-warning"
              : "bg-ink-600"
          }`}
        />
        <span className="text-[12px] font-medium text-ink-100">
          {requestedMicrosandbox
            ? status.ready
              ? "Microsandbox ready"
              : "Microsandbox not ready"
            : "Sandboxing disabled"}
        </span>
        {status.preparePhase ? (
          <span className="ml-auto truncate text-[11px] text-ink-500" title={status.preparePhase}>
            {status.preparePhase}…
          </span>
        ) : null}
      </div>

      {/* Error */}
      {status.error ? (
        <div className="flex items-start gap-2 rounded-lg border border-danger/30 bg-danger/5 px-3 py-2 text-[12px] text-danger">
          <WarningIcon width={14} height={14} className="mt-0.5 shrink-0" />
          <span className="min-w-0 break-words">{status.error}</span>
        </div>
      ) : null}

      {/* Facts */}
      <div className="space-y-1.5 rounded-lg border border-ink-800/80 bg-ink-925/60 px-3 py-2.5">
        <StatusFact
          label="Platform"
          value={
            report
              ? `${report.platform} · ${report.architecture}`
              : status.ready
                ? "detected by core"
                : "run Recheck to detect"
          }
        />
        <StatusFact label="Shell" value={shellHint} />
        <StatusFact
          label="Image"
          value={status.image ?? (requestedMicrosandbox ? "default (pending)" : "—")}
          mono
        />
        <StatusFact
          label="CPU / memory"
          value={
            status.cpus != null && status.memoryMb != null
              ? `${status.cpus} vCPU · ${status.memoryMb} MiB`
              : requestedMicrosandbox
                ? "defaults (pending)"
                : "—"
          }
        />
        <StatusFact
          label="Network"
          value={status.networkMode ?? (requestedMicrosandbox ? "restricted (default)" : "—")}
        />
      </div>

      {/* Setup-required guidance (checks + actions) */}
      {notReady && report ? (
        <div className="space-y-2">
          {report.checks.length > 0 ? (
            <div className="space-y-1.5">
              <div className="text-[11px] font-semibold uppercase tracking-wide text-ink-500">
                Preflight checks
              </div>
              {report.checks.map((c) => {
                const meta = CHECK_STATUS_META[c.status] ?? CHECK_STATUS_META.info;
                return (
                  <div
                    key={c.code}
                    className="flex items-start gap-2 rounded-lg border border-ink-800/80 bg-ink-925/60 px-2.5 py-2"
                  >
                    <span className={`mt-1 h-1.5 w-1.5 shrink-0 rounded-full ${meta.dot}`} />
                    <div className="min-w-0 flex-1">
                      <div className="flex items-baseline gap-2">
                        <span className="text-[12px] font-medium text-ink-100">{c.title}</span>
                        <span className={`text-[10px] uppercase tracking-wide ${meta.className}`}>
                          {meta.label}
                        </span>
                      </div>
                      <p className="mt-0.5 break-words text-[11px] leading-relaxed text-ink-400">
                        {c.detail}
                      </p>
                    </div>
                  </div>
                );
              })}
            </div>
          ) : null}
          {report.actions.length > 0 ? (
            <div className="space-y-1.5">
              <div className="text-[11px] font-semibold uppercase tracking-wide text-ink-500">
                Setup actions
              </div>
              {report.actions.map((a: SandboxSetupAction, i) => (
                <div
                  key={`${a.title}-${i}`}
                  className="space-y-1.5 rounded-lg border border-ink-800/80 bg-ink-925/60 px-2.5 py-2"
                >
                  <div className="flex items-baseline gap-2">
                    <span className="text-[12px] font-medium text-ink-100">{a.title}</span>
                    {a.requires_admin ? (
                      <span className="rounded bg-warning/15 px-1.5 py-0.5 text-[9px] uppercase tracking-wide text-warning">
                        admin
                      </span>
                    ) : null}
                    {a.requires_reboot ? (
                      <span className="rounded bg-warning/15 px-1.5 py-0.5 text-[9px] uppercase tracking-wide text-warning">
                        reboot
                      </span>
                    ) : null}
                  </div>
                  <p className="break-words text-[11px] leading-relaxed text-ink-400">
                    {a.explanation}
                  </p>
                  {a.command ? (
                    <div className="flex items-center gap-2">
                      <code className="min-w-0 flex-1 truncate rounded bg-ink-950 px-2 py-1 font-mono text-[11px] text-ink-300">
                        {a.command}
                      </code>
                      <CopyCommandButton command={a.command} />
                    </div>
                  ) : null}
                </div>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}

      {/* Restart hint: a mode change requires the core to (re)run preflight +
          reconfigure execution routing. */}
      {requestedMicrosandbox && !status.ready && !status.preparePhase ? (
        <p className="flex items-start gap-1.5 text-[11px] leading-relaxed text-ink-500">
          <TerminalIcon width={12} height={12} className="mt-0.5 shrink-0" />
          Agent commands stay on hold until the sandbox is ready. A core restart
          may be required after enabling virtualization.
        </p>
      ) : null}

      {/* Actions */}
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={onRecheck}
          className="flex items-center gap-1.5 rounded-lg border border-ink-700 px-3 py-1.5 text-[12px] font-medium text-ink-200 transition-colors hover:bg-ink-850 hover:text-ink-100"
        >
          <RefreshIcon width={13} height={13} />
          Recheck environment
        </button>
        <button
          type="button"
          onClick={onPrepare}
          disabled={!!status.preparePhase}
          className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-[12px] font-semibold text-white transition-colors hover:bg-accent-soft disabled:cursor-not-allowed disabled:bg-ink-800 disabled:text-ink-500"
        >
          <DownloadIcon width={13} height={13} />
          {status.preparePhase ? "Preparing…" : "Prepare runtime & image"}
        </button>
        {requestedMicrosandbox ? (
          <>
            <button
              type="button"
              onClick={onReset}
              className="flex items-center gap-1.5 rounded-lg border border-ink-700 px-3 py-1.5 text-[12px] font-medium text-ink-300 transition-colors hover:bg-ink-850 hover:text-ink-100"
            >
              <RefreshIcon width={13} height={13} />
              Reset sandbox
            </button>
            <button
              type="button"
              onClick={onDisable}
              className="ml-auto rounded-lg border border-ink-700 px-3 py-1.5 text-[12px] font-medium text-ink-400 transition-colors hover:border-danger/40 hover:text-danger"
            >
              Disable sandboxing
            </button>
          </>
        ) : null}
      </div>
    </Card>
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
                  <FieldLabel>Sandbox</FieldLabel>
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
                            <div className="text-[13px] font-medium">{SANDBOX_LABEL[mode]}</div>
                            <div className="text-[11px] text-ink-500">{SANDBOX_HELP[mode]}</div>
                          </div>
                        </ChoiceButton>
                      );
                    })}
                  </div>
                  <div className="mt-3">
                    <SandboxStatusPanel
                      status={props.sandboxStatus}
                      onRecheck={props.onRecheckSandbox}
                      onPrepare={props.onPrepareSandbox}
                      onReset={props.onResetSandbox}
                      onDisable={() => props.onSetSandbox("none")}
                    />
                  </div>
                  <p className="mt-2 text-[11px] leading-relaxed text-ink-500">
                    When enabled, agent bash, git, diagnostics and plugin scripts
                    run inside a Linux microVM via the Microsandbox Rust SDK —
                    no Docker, Firejail, or external CLI required.
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
