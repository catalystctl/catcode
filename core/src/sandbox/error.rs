//! Structured errors for the sandbox execution subsystem.
//!
//! The key invariant (acceptance criterion #6, #10): when sandboxing is
//! requested but unavailable, execution **fails closed** — it never falls
//! back to running the command on the host. [`ExecutionError::SetupRequired`]
//! carries a full [`SandboxPreflightReport`] so the UI can render exact setup
//! guidance, and callers convert it into a user-facing error rather than
//! silently degrading.
use serde::Serialize;

/// A single preflight check result.
#[derive(Clone, Debug, Serialize)]
pub struct SandboxPreflightCheck {
    /// Stable machine-readable code (see `error_codes` below), e.g. `kvm_device_missing`.
    pub code: String,
    /// Short human title.
    pub title: String,
    /// Pass / fail / warn / info.
    pub status: CheckStatus,
    /// Human-readable detail / value.
    pub detail: String,
}

/// Status of a preflight check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Fail,
    Warn,
    Info,
}

/// A user-actionable setup step (administrator or user-space).
#[derive(Clone, Debug, Serialize)]
pub struct SandboxSetupAction {
    /// Short title.
    pub title: String,
    /// Why this step is needed / what it does.
    pub explanation: String,
    /// A copyable command, when one exists.
    pub command: Option<String>,
    /// Whether this command requires administrator / root / elevated PowerShell.
    pub requires_admin: bool,
    /// Whether a reboot / re-login is required for the change to take effect.
    pub requires_reboot: bool,
}

/// The full environment-readiness report produced by preflight. Serialized to
/// the UI over the protocol (`sandbox_status` / `sandbox_setup_required`).
#[derive(Clone, Debug, Default, Serialize)]
pub struct SandboxPreflightReport {
    /// Whether the user requested sandboxing.
    pub requested: bool,
    /// Whether the platform/arch is supported at all by the pinned SDK release.
    pub supported: bool,
    /// Whether the environment is ready to boot a sandbox right now.
    pub ready: bool,
    /// OS string (linux / macos / windows).
    pub platform: String,
    /// Architecture string (x86_64 / aarch64 / x86).
    pub architecture: String,
    /// Individual checks.
    pub checks: Vec<SandboxPreflightCheck>,
    /// Actionable setup steps (admin + user-space).
    pub actions: Vec<SandboxSetupAction>,
}

impl SandboxPreflightReport {
    pub fn blocking_code(&self) -> Option<&str> {
        self.checks
            .iter()
            .find(|c| c.status == CheckStatus::Fail)
            .map(|c| c.code.as_str())
    }
}

/// Errors returned by the execution backend. Never carries secret values.
#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    /// The host process failed to spawn (binary missing on PATH, permission, …).
    #[error("{context}: {message}")]
    SpawnFailed { context: String, message: String },

    /// A required executable is missing inside the sandbox guest image. Carries
    /// an actionable message naming the binary, the active image, and how to
    /// select a different image. Never falls back to the host compiler.
    #[error("{message}")]
    MissingExecutable { message: String },

    /// The command exceeded its timeout. The guest process (and descendants)
    /// has been terminated; if termination could not be confirmed the sandbox
    /// was reset.
    #[error("command timed out after {0:?} and was killed")]
    Timeout(std::time::Duration),

    /// The sandbox failed to initialize or execute (boot failure, guest agent
    /// unavailable, image pull failure, …). Carries a stable code.
    #[error("sandbox error ({code}): {message}")]
    Sandbox { code: String, message: String },

    /// Sandboxing was requested but the environment is not ready. Carries the
    /// full preflight report. Callers must surface this to the user and NOT
    /// execute on the host.
    #[error("sandbox setup required")]
    SetupRequired { report: SandboxPreflightReport },

    /// Generic I/O or runtime error.
    #[error("{0}")]
    Other(String),
}

impl ExecutionError {
    pub fn spawn_failed(program: &str, e: std::io::Error) -> Self {
        ExecutionError::SpawnFailed {
            context: format!("failed to spawn {program:?}"),
            message: e.to_string(),
        }
    }

    /// Build an actionable "missing executable" error for a sandboxed run.
    pub fn missing_executable(name: &str, image: &str) -> Self {
        ExecutionError::MissingExecutable {
            message: format!(
                "The executable `{name}` is not available inside the sandbox image `{image}`.\n\
                 CatCode does not fall back to host tools when sandboxing is enabled.\n\
                 To fix this, select a sandbox image that includes `{name}`:\n  \
                 - set `sandbox_image` in your config to a CatCode image variant that bundles the toolchain, or\n  \
                 - build/publish a project-specific image and point `sandbox_image` at it.\n\
                 See docs/guides/sandbox.md for the image strategy and the list of bundled tools."
            ),
        }
    }

    /// A single-line user-facing summary suitable for a tool `Outcome::err`.
    pub fn user_message(&self) -> String {
        match self {
            ExecutionError::SetupRequired { report } => {
                let code = report.blocking_code().unwrap_or("unknown");
                let title = report
                    .checks
                    .iter()
                    .find(|c| c.status == CheckStatus::Fail)
                    .map(|c| c.title.as_str())
                    .unwrap_or("Sandbox not ready");
                let mut s = format!(
                    "sandboxing is enabled but the environment is not ready ({code}: {title}). "
                );
                if let Some(a) = report.actions.first() {
                    s.push_str(&format!("Next step: {}. ", a.title));
                    if let Some(cmd) = &a.command {
                        s.push_str(&format!("Run: `{cmd}`. "));
                    }
                }
                s.push_str("Run `/sandbox status` for details, or disable sandboxing with `--sandbox none`.");
                s
            }
            ExecutionError::MissingExecutable { message } => message.clone(),
            ExecutionError::Sandbox { code, message } => {
                format!("sandbox error ({code}): {message}")
            }
            other => other.to_string(),
        }
    }
}

/// Stable, machine-readable preflight error codes. Surfaced in
/// [`SandboxPreflightCheck::code`] and [`ExecutionError::Sandbox::code`].
#[allow(dead_code)]
pub mod error_codes {
    pub const UNSUPPORTED_PLATFORM: &str = "unsupported_platform";
    pub const UNSUPPORTED_ARCHITECTURE: &str = "unsupported_architecture";
    pub const VIRTUALIZATION_DISABLED: &str = "virtualization_disabled";
    pub const NESTED_VIRTUALIZATION_UNAVAILABLE: &str = "nested_virtualization_unavailable";
    pub const KVM_DEVICE_MISSING: &str = "kvm_device_missing";
    pub const KVM_PERMISSION_DENIED: &str = "kvm_permission_denied";
    pub const KVM_MODULES_MISSING: &str = "kvm_modules_missing";
    pub const WHP_DISABLED: &str = "whp_disabled";
    pub const WHP_RESTART_PENDING: &str = "whp_restart_pending";
    pub const INTEL_MACOS_UNSUPPORTED: &str = "intel_macos_unsupported";
    pub const RUNTIME_MISSING: &str = "runtime_missing";
    pub const RUNTIME_DOWNLOAD_FAILED: &str = "runtime_download_failed";
    pub const IMAGE_PULL_REQUIRED: &str = "image_pull_required";
    pub const IMAGE_PULL_FAILED: &str = "image_pull_failed";
    pub const SANDBOX_BOOT_FAILED: &str = "sandbox_boot_failed";
    pub const GUEST_AGENT_UNAVAILABLE: &str = "guest_agent_unavailable";
    pub const SDK_NOT_COMPILED: &str = "sdk_not_compiled";
    pub const TIMEOUT: &str = "timeout";
    pub const CANCELLED: &str = "cancelled";
}
