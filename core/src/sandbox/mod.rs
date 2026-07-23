//! CatCode sandbox subsystem.
//!
//! Replaces the legacy Firejail / Seatbelt (`sandbox-exec`) / `unshare -n`
//! backends with a single Microsandbox microVM execution model. The user never
//! installs Docker, Podman, Firejail, WSL, the `msb` CLI, or a persistent daemon
//! — the embedded SDK downloads its own runtime on first use.
//!
//! Architecture:
//!   - [`backend`] defines the CatCode-owned [`ExecutionBackend`] trait +
//!     [`ExecRequest`]/[`ExecResult`] + [`HostExecutionBackend`].
//!   - [`microsandbox_backend`] (feature `microsandbox`) is the ONLY module that
//!     touches the `microsandbox` SDK.
//!   - [`manager`] selects the backend from config and exposes lifecycle ops.
//!   - [`policy`] holds the security invariants (guest env, secret denial,
//!     workspace path confinement, shell selection, network policy).
//!   - [`preflight`] is the injectable environment-readiness check.
//!   - [`error`] carries structured setup-required reports (fail-closed).
//!
//! The active backend is a process-global set once at startup from config
//! (see [`init_from_config`]). Tools obtain it via [`execution_backend`]; it
//! defaults to the host backend (so tests with `Sandbox::None` work unchanged).
pub mod backend;
pub mod error;
pub mod manager;
#[cfg(feature = "microsandbox")]
pub mod microsandbox_backend;
pub mod policy;
pub mod preflight;
mod setup;

use std::sync::{Arc, OnceLock};

use crate::config::Config;

use backend::ExecutionBackend;

static EXEC: OnceLock<Arc<dyn ExecutionBackend>> = OnceLock::new();
/// The loaded config (set at init). Used by call sites that need workspace /
/// plugin-dir paths for sandbox path translation but don't otherwise hold a
/// `&Config` (e.g. plugin hook launchers). Defaults to a fresh `Config` when
/// unset (host mode in unit tests).
static CONFIG: OnceLock<Arc<Config>> = OnceLock::new();

/// Install the process-global execution backend + config. Called once at core
/// startup from the loaded config.
pub fn init_from_config(cfg: Arc<Config>) {
    let _ = CONFIG.set(cfg.clone());
    let backend = manager::select_backend(cfg);
    let _ = EXEC.set(backend);
}

/// The loaded config (or a default when unset — e.g. unit tests).
pub fn config() -> Arc<Config> {
    CONFIG
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(Config::default()))
}

/// The active execution backend. Defaults to a [`backend::HostExecutionBackend`]
/// when unset (e.g. in unit tests) so `Sandbox::None` paths behave unchanged.
pub fn execution_backend() -> Arc<dyn ExecutionBackend> {
    EXEC.get()
        .cloned()
        .unwrap_or_else(|| Arc::new(backend::HostExecutionBackend))
}

/// Whether the active backend runs commands inside a microVM.
pub fn is_sandbox_enabled() -> bool {
    execution_backend().is_sandboxed()
}

/// Reset the active sandbox (no-op for the host backend).
pub async fn reset_sandbox() -> Result<(), error::ExecutionError> {
    execution_backend().reset().await
}

/// Prepare runtime/image assets for the active backend (no-op for host).
pub async fn prepare_sandbox() -> Result<(), error::ExecutionError> {
    execution_backend().prepare().await
}

/// Current preflight report for the active backend.
pub async fn sandbox_status() -> error::SandboxPreflightReport {
    execution_backend().status().await
}

/// Cleanly stop the active sandbox. Called on core shutdown.
pub async fn shutdown() {
    execution_backend().shutdown().await;
}

pub use backend::{ExecRequest, ExecResult, HostExecutionBackend};
pub use error::{
    CheckStatus, ExecutionError, SandboxPreflightCheck, SandboxPreflightReport, SandboxSetupAction,
};
pub use manager::{select_backend, SandboxManager};
pub use policy::{is_sandbox_enabled as is_enabled, ShellKind};
