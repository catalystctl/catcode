//! Sandbox manager: backend selection (factory) + lifecycle ops.
//!
//! [`SandboxManager::select_backend`] is the single place that decides host vs
//! microVM based on `Config::sandbox`. The chosen backend is installed as the
//! process-global backend ([`super::set_execution_backend`]) at startup.
use std::sync::Arc;

use crate::config::{Config, Sandbox};

use super::backend::{ExecutionBackend, HostExecutionBackend};
use super::error::ExecutionError;
use super::error::SandboxPreflightReport;
use super::preflight::{run_platform_preflight, RealProbe};

/// Selects the execution backend for a config.
///
/// `Sandbox::None` → [`HostExecutionBackend`].
/// `Sandbox::Microsandbox` → [`MicrosandboxExecutionBackend`] (when the
/// `microsandbox` feature is compiled in); otherwise a backend that always
/// fails closed with a `sdk_not_compiled` setup-required error.
pub fn select_backend(cfg: Arc<Config>) -> Arc<dyn ExecutionBackend> {
    match cfg.sandbox {
        Sandbox::None => Arc::new(HostExecutionBackend),
        Sandbox::Microsandbox => {
            #[cfg(feature = "microsandbox")]
            {
                return Arc::new(
                    super::microsandbox_backend::MicrosandboxExecutionBackend::new(cfg),
                );
            }
            #[cfg(not(feature = "microsandbox"))]
            {
                let _ = cfg;
                Arc::new(UnsupportedSandboxBackend)
            }
        }
    }
}

/// Fail-closed backend used when sandboxing is requested but the `microsandbox`
/// feature was not compiled in. Every execute returns a structured setup-required
/// error; it never runs on the host.
#[cfg(not(feature = "microsandbox"))]
struct UnsupportedSandboxBackend;

#[cfg(not(feature = "microsandbox"))]
#[async_trait::async_trait]
impl ExecutionBackend for UnsupportedSandboxBackend {
    fn is_sandboxed(&self) -> bool {
        true
    }
    async fn execute(
        &self,
        _request: super::backend::ExecRequest,
    ) -> Result<super::backend::ExecResult, ExecutionError> {
        Err(unsupported_report())
    }
    async fn status(&self) -> SandboxPreflightReport {
        match unsupported_report() {
            ExecutionError::SetupRequired { report } => report,
            _ => run_platform_preflight(&RealProbe, true),
        }
    }
}

#[cfg(not(feature = "microsandbox"))]
fn unsupported_report() -> ExecutionError {
    let mut report = run_platform_preflight(&RealProbe, true);
    report.checks.push(super::error::SandboxPreflightCheck {
        code: super::error::error_codes::SDK_NOT_COMPILED.to_string(),
        title: "Microsandbox SDK".to_string(),
        status: super::error::CheckStatus::Fail,
        detail: "this CatCode build was compiled without the `microsandbox` feature".to_string(),
    });
    report.ready = false;
    ExecutionError::SetupRequired { report }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::time::Duration;

    /// When sandboxing is requested but the `microsandbox` feature is not
    /// compiled in, the backend must FAIL CLOSED — every execute returns a
    /// structured setup-required error and never runs on the host.
    #[cfg(not(feature = "microsandbox"))]
    #[tokio::test]
    async fn fail_closed_when_feature_off_never_runs_on_host() {
        let mut cfg = Config::default();
        cfg.sandbox = Sandbox::Microsandbox;
        let backend = select_backend(Arc::new(cfg));
        assert!(
            backend.is_sandboxed(),
            "requested microsandbox must be sandboxed"
        );
        let req = super::super::backend::ExecRequest {
            program: "echo".to_string(),
            args: vec!["hello".to_string()],
            cwd: std::env::temp_dir(),
            env: Default::default(),
            inherit_parent_env: false,
            stdin: None,
            timeout: Duration::from_secs(5),
            ..Default::default()
        };
        let result = backend.execute(req).await;
        assert!(
            result.is_err(),
            "must NOT execute on the host when sandboxing is requested but unavailable"
        );
        match result {
            Err(ExecutionError::SetupRequired { .. }) => {}
            other => panic!("expected SetupRequired, got {other:?}"),
        }
    }

    /// When sandbox=none, the host backend runs (unchanged behavior).
    #[cfg(not(feature = "microsandbox"))]
    #[tokio::test]
    async fn host_backend_when_disabled() {
        let cfg = Config::default(); // sandbox = None
        let backend = select_backend(Arc::new(cfg));
        assert!(!backend.is_sandboxed());
        assert!(!backend.is_sandboxed());
    }

    /// Real-VM integration: only runs when CATCODE_TEST_MICROSANDBOX=1 is set.
    /// Verifies the sandbox executes a command in the guest and that a timed-out
    /// command is killed (not left running). Without the env var it is a no-op.
    #[cfg(feature = "microsandbox")]
    #[tokio::test]
    async fn real_vm_exec_and_timeout() {
        if std::env::var("CATCODE_TEST_MICROSANDBOX").as_deref() != Ok("1") {
            eprintln!(
                "[sandbox] skipping real-VM test (set CATCODE_TEST_MICROSANDBOX=1 to enable)"
            );
            return;
        }
        let mut cfg = Config::default();
        cfg.sandbox = Sandbox::Microsandbox;
        let backend = select_backend(Arc::new(cfg));
        let req = super::super::backend::ExecRequest {
            program: "bash".to_string(),
            args: vec!["-c".to_string(), "echo guest-ok".to_string()],
            cwd: std::path::PathBuf::from("/workspace"),
            env: Default::default(),
            inherit_parent_env: false,
            stdin: None,
            timeout: Duration::from_secs(120),
            ..Default::default()
        };
        let out = backend
            .execute(req)
            .await
            .expect("sandbox exec should succeed when CATCODE_TEST_MICROSANDBOX=1");
        assert_eq!(out.exit_code, Some(0));
        assert!(String::from_utf8_lossy(&out.stdout).contains("guest-ok"));

        // A timed-out command must be killed, not left running.
        let slow = super::super::backend::ExecRequest {
            program: "bash".to_string(),
            args: vec!["-c".to_string(), "sleep 30".to_string()],
            cwd: std::path::PathBuf::from("/workspace"),
            env: Default::default(),
            inherit_parent_env: false,
            stdin: None,
            timeout: Duration::from_secs(2),
            ..Default::default()
        };
        let slow_out = backend.execute(slow).await;
        assert!(
            slow_out.is_err() || slow_out.as_ref().unwrap().timed_out,
            "sleep 30 must time out (killed), not run to completion"
        );
    }
}
