//! The CatCode-owned execution abstraction.
//!
//! All agent-controlled process execution goes through [`ExecutionBackend`].
//! There are exactly two implementations:
//!   - [`HostExecutionBackend`] — selected only when sandbox mode is `none`.
//!   - [`MicrosandboxExecutionBackend`] (feature `microsandbox`) — runs the
//!     command inside a Microsandbox microVM.
//!
//! Tools, plugins, Git helpers, diagnostics, and test_env never spawn
//! `tokio::process::Command` directly anymore; they build an [`ExecRequest`]
//! and call [`execution_backend`](super::execution_backend). This keeps the
//! Microsandbox SDK surface confined to `microsandbox_backend.rs`.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;

use super::error::ExecutionError;

/// A request to run a program. The backend decides *where* (host or microVM).
#[derive(Clone, Debug)]
pub struct ExecRequest {
    /// Program to run (resolved by the backend: host PATH or guest PATH).
    pub program: String,
    /// Argv (excluding the program).
    pub args: Vec<String>,
    /// Working directory. For the microVM this is a *guest* path (e.g.
    /// `/workspace`); the manager/`policy::effective_cwd` produces the right
    /// value so callers always pass the resolved path for the active backend.
    pub cwd: PathBuf,
    /// Environment to apply. For the host backend this is the full minimal env
    /// (the backend does `env_clear` then sets these) unless
    /// `inherit_parent_env` is true. For the microVM these merge on top of the
    /// sandbox-level guest env; callers pass an empty map for the base guest
    /// env (set at sandbox creation).
    pub env: BTreeMap<String, String>,
    /// Host-only: when true, do NOT `env_clear` (Windows PowerShell path that
    /// depends on SystemRoot/PATHEXT/APPDATA). Always false under the microVM.
    pub inherit_parent_env: bool,
    /// Stdin bytes, or None for /dev/null.
    pub stdin: Option<Vec<u8>>,
    /// Wall-clock timeout. On expiry the process (and descendants) is killed.
    pub timeout: Duration,
    /// Hard cap on captured stdout (keeps the tail). Safety net against OOM;
    /// tools apply their own 32 KiB smart truncation afterwards.
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
}

impl Default for ExecRequest {
    fn default() -> Self {
        Self {
            program: String::new(),
            args: Vec::new(),
            cwd: PathBuf::from("."),
            env: BTreeMap::new(),
            inherit_parent_env: false,
            stdin: None,
            timeout: Duration::from_secs(30),
            max_stdout_bytes: 8 * 1024 * 1024,
            max_stderr_bytes: 8 * 1024 * 1024,
        }
    }
}

/// The result of a completed (or killed) execution.
#[derive(Clone, Debug, Default)]
pub struct ExecResult {
    /// Exit code if the process exited normally.
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// True if the command was killed because it exceeded its timeout.
    pub timed_out: bool,
}

impl ExecResult {
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// A unified async execution interface. Implementations must guarantee that a
/// dropped [`ExecRequest`] future terminates the guest process and its
/// descendants (the microVM backend does this explicitly via the SDK control
/// handle; a bare `drop` is not relied upon).
#[async_trait]
pub trait ExecutionBackend: Send + Sync {
    /// Execute the request. On a sandbox setup/availability failure this
    /// returns [`ExecutionError::SetupRequired`] and the caller MUST NOT fall
    /// back to the host.
    async fn execute(&self, request: ExecRequest) -> Result<ExecResult, ExecutionError>;
    /// Reset the backing sandbox (no-op for the host backend). Used after an
    /// unhealthy execution or when the user requests `/sandbox reset`.
    async fn reset(&self) -> Result<(), ExecutionError> {
        Ok(())
    }
    /// Prepare runtime/image assets (downloads, image pull). No-op for the host
    /// backend. Returns a fresh preflight report on failure.
    async fn prepare(&self) -> Result<(), ExecutionError> {
        Ok(())
    }
    /// Current readiness + preflight report (best-effort, never errors).
    async fn status(&self) -> super::error::SandboxPreflightReport {
        super::error::SandboxPreflightReport::default()
    }
    /// Cleanly stop the backing sandbox (no-op for the host backend).
    async fn shutdown(&self) {}
    /// `"host"` or `"microsandbox"`.
    fn label(&self) -> &'static str {
        "host"
    }
    /// Whether this backend runs commands inside a microVM.
    fn is_sandboxed(&self) -> bool {
        false
    }
}

/// Run commands directly on the host. Selected ONLY when `Sandbox::None`.
///
/// Reproduces the pre-migration host hygiene: `env_clear` + minimal env on
/// POSIX shells (no LD_PRELOAD / proxy leak), inherit-parent on Windows
/// PowerShell, `kill_on_drop`, bounded timeout, output truncation.
#[derive(Clone, Debug, Default)]
pub struct HostExecutionBackend;

#[async_trait]
impl ExecutionBackend for HostExecutionBackend {
    async fn execute(&self, request: ExecRequest) -> Result<ExecResult, ExecutionError> {
        let mut cmd = tokio::process::Command::new(&request.program);
        cmd.args(&request.args);
        if request.cwd.as_os_str() != "." {
            cmd.current_dir(&request.cwd);
        }
        if !request.inherit_parent_env {
            cmd.env_clear();
        }
        for (k, v) in &request.env {
            cmd.env(k, v);
        }
        cmd.stdin(if request.stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        });
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| ExecutionError::spawn_failed(&request.program, e))?;

        // Feed stdin (small, e.g. a sudo password) then close → EOF.
        if let Some(input) = request.stdin.clone() {
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(&input).await;
                // drop stdin → pipe closes → child sees EOF.
            }
        }

        let result = tokio::time::timeout(request.timeout, child.wait_with_output()).await;
        match result {
            Ok(Ok(o)) => Ok(ExecResult {
                exit_code: o.status.code(),
                stdout: truncate_tail(&o.stdout, request.max_stdout_bytes),
                stderr: truncate_tail(&o.stderr, request.max_stderr_bytes),
                timed_out: false,
            }),
            Ok(Err(e)) => Err(ExecutionError::Other(format!("wait failed: {e}"))),
            Err(_) => {
                // Timeout: kill_on_drop(true) terminates the child when the
                // dropped future reaps it. Return a deterministic timeout error.
                Err(ExecutionError::Timeout(request.timeout))
            }
        }
    }
}

/// Keep the tail (where errors usually are) when over the byte cap.
pub(crate) fn truncate_tail(data: &[u8], max: usize) -> Vec<u8> {
    if data.len() <= max {
        data.to_vec()
    } else {
        let start = data.len() - max;
        let mut out = b"...[truncated]...\n".to_vec();
        out.extend_from_slice(&data[start..]);
        out
    }
}
