// Sandbox types + helpers for the Microsandbox migration.
//
// The Rust `core` is the source of truth for sandbox behavior. This module only
// mirrors the protocol contract the core already implements (verified against
// `core/src/config.rs`, `core/src/sandbox/**`, and `core/src/commands/dispatcher.rs`):
//
//   - `sandbox` option values: "none" | "microsandbox" (wire form).
//   - Legacy backends (firejail/seatbelt/sandbox-exec/unshare) are gone. The
//     core still *accepts* their old spellings as aliases for microsandbox
//     (with a deprecation notice), but the SDK must NOT emit them — it
//     normalizes to "microsandbox" before spawning the core.
//   - `ready` carries: sandbox, shell, sandboxImage, sandboxCpus,
//     sandboxMemoryMb, sandboxNetworkMode, sandboxReady.
//   - New commands: get_sandbox_status / prepare_sandbox / reset_sandbox.
//   - New events: sandbox_status / sandbox_prepare_progress / sandbox_ready /
//     sandbox_error. `report` is a serialized SandboxPreflightReport (snake_case
//     fields, matching `core/src/sandbox/error.rs`).

// ── Sandbox mode ──

/** Operational sandbox modes the core actually understands on the wire. */
export type SandboxMode = "none" | "microsandbox";

/**
 * Legacy sandbox backend spellings the SDK still ACCEPTS as input (for source
 * compatibility with pre-migration callers) but never EMITS to the core. Each
 * is normalized to {@link SandboxMode} "microsandbox" with a deprecation
 * notice. They are NOT operational backends.
 *
 * @deprecated Pass `"microsandbox"` (or `"none"`) instead.
 */
export type SandboxLegacyAlias =
  | "firejail"
  | "fj"
  | "seatbelt"
  | "macos"
  | "sandbox-exec";

/**
 * Accepted `sandbox` option input. Strict callers should use {@link SandboxMode}
 * directly; the legacy aliases remain only to keep existing code compiling.
 */
export type SandboxOption = SandboxMode | SandboxLegacyAlias;

const NONE_ALIASES = new Set<string>(["none", "off", "false", "disabled", "disable"]);
const MICROSANDBOX_ALIASES = new Set<string>([
  "microsandbox",
  "msb",
  "on",
  "true",
  "enabled",
  "enable",
]);
const LEGACY_ALIASES = new Set<string>(["firejail", "fj", "seatbelt", "macos", "sandbox-exec"]);

/** Names of legacy aliases we have already deprecation-warned about (once each). */
const warnedLegacy = new Set<string>();

/**
 * Normalize a sandbox option to a wire {@link SandboxMode}.
 *
 * - `"none"` (and off/false/disabled) → `"none"`.
 * - `"microsandbox"` (and msb/on/true/enabled) → `"microsandbox"`.
 * - Legacy spellings (firejail/fj/seatbelt/macos/sandbox-exec) →
 *   `"microsandbox"` with a one-time deprecation notice. The user's *intent to
 *   enable sandboxing* is preserved; it is never silently downgraded to none.
 * - Unknown values throw — unrecognized sandbox input is a programming error
 *   and the SDK fails closed rather than guessing (the core itself would
 *   silently treat unknown as `none`, which we do not replicate).
 */
export function normalizeSandboxMode(input: SandboxOption | string | undefined): SandboxMode | undefined {
  if (input === undefined) return undefined;
  const value = String(input).toLowerCase();
  if (value === "") return undefined;
  if (NONE_ALIASES.has(value)) return "none";
  if (MICROSANDBOX_ALIASES.has(value)) return "microsandbox";
  if (LEGACY_ALIASES.has(value)) {
    if (!warnedLegacy.has(value)) {
      warnedLegacy.add(value);
      // eslint-disable-next-line no-console
      console.warn(
        `[catcode-sdk] The "${value}" sandbox backend has been replaced by ` +
          `Microsandbox. Continuing with sandbox="microsandbox". Update your ` +
          `code to use sandbox: "microsandbox" (or "none") to silence this notice.`,
      );
    }
    return "microsandbox";
  }
  throw new RangeError(
    `Unknown sandbox mode "${input}". Use "none" or "microsandbox" (legacy ` +
      `aliases firejail/fj/seatbelt/macos/sandbox-exec map to "microsandbox").`,
  );
}

// ── Network policy ──

/**
 * Guest network egress policy (mirrors `core/src/config.rs::SandboxNetworkMode`).
 *
 * - `none`        — no network interface (the legacy `--no-network` behavior).
 * - `restricted`  — network up; cloud-metadata/host-only/private ranges blocked
 *                   by default. Default mode.
 * - `allowlist`   — network up; only entries in `networkAllowlist` reachable.
 */
export type SandboxNetworkMode = "none" | "restricted" | "allowlist";

// ── Sandbox configuration ──

/**
 * Typed sandbox configuration. Mirrors the core's config fields
 * (`core/src/config.rs::Config` sandbox_*). Fields without a core CLI flag
 * (`networkAllowlist`, `allowPrivateNetworks`, `idleTimeoutSecs`) are applied
 * by the core via config file / environment only; the SDK documents this
 * rather than inventing unsupported CLI flags.
 */
export interface SandboxConfig {
  /** OCI image reference for the sandbox guest. */
  image?: string;
  /** Guest vCPUs (core clamps to 1..16). */
  cpus?: number;
  /** Guest memory in MiB (core enforces safe min/max). */
  memoryMb?: number;
  /** Guest writable overlay in MiB. */
  diskMb?: number;
  /** Guest network egress policy. */
  networkMode?: SandboxNetworkMode;
  /**
   * Hosts/CIDRs reachable in `allowlist` mode. Config-file/env only — no core
   * CLI flag. Pass via the core config file or `CATALYST_CODE_*` env if needed.
   */
  networkAllowlist?: string[];
  /**
   * Whether to permit private network ranges. Config-file/env only. Default
   * `false`.
   */
  allowPrivateNetworks?: boolean;
  /**
   * Guest environment-variable allowlist (in addition to the minimal default
   * set PATH/HOME/LANG/...). Emits `--sandbox-env-allowlist <csv>`.
   */
  envAllowlist?: string[];
  /**
   * Idle timeout (seconds) before an unused sandbox is stopped. Config-file/env
   * only — no core CLI flag.
   */
  idleTimeoutSecs?: number;
}

// ── Preflight report (mirrors core/src/sandbox/error.rs, snake_case on wire) ──

/** Status of an individual preflight check. Serialized lowercase by the core. */
export type SandboxPreflightCheckStatus = "pass" | "fail" | "warn" | "info";

/** A single preflight check result. */
export interface SandboxPreflightCheck {
  /** Stable machine-readable code (e.g. `kvm_device_missing`). */
  code: string;
  /** Short human title. */
  title: string;
  /** Pass / fail / warn / info. */
  status: SandboxPreflightCheckStatus;
  /** Human-readable detail / value. */
  detail: string;
}

/** A user-actionable setup step (administrator or user-space). */
export interface SandboxSetupAction {
  /** Short title. */
  title: string;
  /** Why this step is needed / what it does. */
  explanation: string;
  /** A copyable command, when one exists. */
  command?: string | null;
  /** Whether this command requires administrator / root / elevated PowerShell. */
  requires_admin: boolean;
  /** Whether a reboot / re-login is required for the change to take effect. */
  requires_reboot: boolean;
}

/**
 * Full environment-readiness report produced by preflight. Field names are
 * snake_case to match the core's serde serialization exactly.
 */
export interface SandboxPreflightReport {
  /** Whether the user requested sandboxing. */
  requested: boolean;
  /** Whether the platform/arch is supported at all by the pinned SDK release. */
  supported: boolean;
  /** Whether the environment is ready to boot a sandbox right now. */
  ready: boolean;
  /** OS string (linux / macos / windows). */
  platform: string;
  /** Architecture string (x86_64 / aarch64 / x86). */
  architecture: string;
  /** Individual checks. */
  checks: SandboxPreflightCheck[];
  /** Actionable setup steps (admin + user-space). */
  actions: SandboxSetupAction[];
}

/**
 * Status returned by `get_sandbox_status` / `reset_sandbox` (the
 * `sandbox_status` event payload). `report` may be absent if the core could
 * not build it.
 */
export interface SandboxStatus {
  /** Effective sandbox mode: "none" | "microsandbox". */
  mode: SandboxMode;
  /** Full preflight report (absent only on core error). */
  report?: SandboxPreflightReport;
  /** True when this status follows a `reset_sandbox` command. */
  reset?: boolean;
}

/** Terminal `sandbox_ready` event payload (success of `prepare_sandbox`). */
export interface SandboxReady {
  /** Whether the sandbox is now ready to execute commands. */
  ready: boolean;
  /** Full preflight report. */
  report?: SandboxPreflightReport;
}

/** Terminal `sandbox_error` event payload (failure of `prepare_sandbox`). */
export interface SandboxError {
  /** Human-readable error message (never carries secret values). */
  error: string;
}
