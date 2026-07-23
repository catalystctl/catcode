//! Sandbox policy: guest environment construction, secret denial, network-mode
//! translation, shell selection, and workspace path confinement.
//!
//! These are the security invariants the task demands:
//!   - No host environment inheritance (minimal guest env, secrets denied).
//!   - `Approval::Never` must NOT disable workspace file confinement.
//!   - Workspace mapped to `/workspace`; no host home / `.ssh` / sockets.
//!   - `--no-network` enforced through Microsandbox network policy.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::{Config, Sandbox, SandboxNetworkMode};

/// Which shell the `bash` tool runs commands in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    /// POSIX `bash -c <command>` (Unix host, or any host when sandboxed — the
    /// guest is Linux).
    Posix,
    /// Windows PowerShell (`-NoProfile -NonInteractive -Command`).
    PowerShell,
}

impl ShellKind {
    pub fn is_posix(self) -> bool {
        matches!(self, ShellKind::Posix)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            ShellKind::Posix => "bash",
            ShellKind::PowerShell => "powershell",
        }
    }
}

/// Read whether sandboxing is enabled from the active backend (single source of
/// truth — the global backend is set at startup from config).
pub fn is_sandbox_enabled() -> bool {
    super::execution_backend().is_sandboxed()
}

/// The effective shell kind for the `bash` tool. When sandboxed, the guest is
/// always Linux `bash` (POSIX), so Windows users are no longer told to emit
/// PowerShell. When unsandboxed, it follows the host-native shell.
pub fn effective_shell_kind() -> ShellKind {
    if is_sandbox_enabled() {
        return ShellKind::Posix;
    }
    host_shell_kind()
}

/// Host-native shell kind (ignores sandbox state). Mirrors `tools::shell_is_posix`.
fn host_shell_kind() -> ShellKind {
    let prog = resolve_host_shell();
    let stem = Path::new(&prog)
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if matches!(
        stem.as_str(),
        "bash" | "sh" | "zsh" | "dash" | "ksh" | "ash" | "busybox"
    ) {
        ShellKind::Posix
    } else {
        ShellKind::PowerShell
    }
}

/// Resolve the host shell program (CATALYST_CODE_SHELL override or OS default).
pub(crate) fn resolve_host_shell() -> String {
    if let Ok(s) = std::env::var("CATALYST_CODE_SHELL") {
        let s = s.trim();
        if !s.is_empty() {
            return s.to_string();
        }
    }
    #[cfg(target_os = "windows")]
    {
        if crate::tools::pwsh_available() {
            "pwsh".to_string()
        } else {
            "powershell".to_string()
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        "bash".to_string()
    }
}

/// Build `(program, args)` to run a single command string in the active shell.
/// POSIX: `<shell> -c <command>`. PowerShell: `-NoProfile -NonInteractive
/// -Command`. When sandboxed the program is always `bash` (resolved in the
/// guest), never a Windows host shell path.
pub fn shell_argv(command: &str) -> (String, Vec<String>) {
    let kind = effective_shell_kind();
    match kind {
        ShellKind::Posix => (
            "bash".to_string(),
            vec!["-c".to_string(), command.to_string()],
        ),
        ShellKind::PowerShell => {
            let prog = resolve_host_shell();
            (
                prog,
                vec![
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    command.into(),
                ],
            )
        }
    }
}

/// Purpose of an exec — selects which host env extras (compiler caches) to
/// include when running unsandboxed. Under the microVM the guest image owns its
/// own toolchains, so no host extras are forwarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecPurpose {
    Bash,
    Diagnostics,
    Git,
    Plugin,
    TestEnv,
    Generic,
}

impl ExecPurpose {
    fn host_extras(&self) -> &'static [&'static str] {
        match self {
            ExecPurpose::Diagnostics => &[
                "CARGO_HOME",
                "RUSTUP_HOME",
                "GOPATH",
                "GOCACHE",
                "GOTMPDIR",
                "NODE_PATH",
                "npm_config_cache",
            ],
            _ => &[],
        }
    }
}

/// Result of env preparation: the env map to apply plus whether the host
/// backend should inherit the parent environment (Windows PowerShell path).
#[derive(Clone, Debug, Default)]
pub struct ProcessEnv {
    pub env: BTreeMap<String, String>,
    pub inherit_parent: bool,
}

/// Build the per-command environment for the active backend.
///
/// - **Host, POSIX shell:** `env_clear` + PATH/HOME/TMPDIR/USER (+ purpose
///   extras). No LD_PRELOAD / proxy leak.
/// - **Host, PowerShell:** inherit the parent env (SystemRoot/PATHEXT/APPDATA
///   are required); apply nothing extra.
/// - **Microsandbox:** empty per-command map — the base guest env is set at
///   sandbox creation (see [`guest_base_env`]); secrets are never inherited.
pub fn build_process_env(cfg: &Config, purpose: ExecPurpose) -> ProcessEnv {
    if is_sandbox_enabled() {
        return ProcessEnv::default();
    }
    let kind = host_shell_kind();
    if !kind.is_posix() {
        // PowerShell depends on the Windows process environment.
        return ProcessEnv {
            env: BTreeMap::new(),
            inherit_parent: true,
        };
    }
    let mut env = BTreeMap::new();
    env.insert(
        "PATH".to_string(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
    );
    if let Ok(home) = std::env::var("HOME") {
        env.insert("HOME".into(), home);
    }
    if let Ok(tmp) = std::env::var("TMPDIR") {
        env.insert("TMPDIR".into(), tmp);
    }
    if let Ok(user) = std::env::var("USER") {
        env.insert("USER".into(), user);
    }
    for k in purpose.host_extras() {
        if let Ok(v) = std::env::var(k) {
            env.insert((*k).into(), v);
        }
    }
    let _ = cfg;
    ProcessEnv {
        env,
        inherit_parent: false,
    }
}

/// The minimal guest environment baked into every Microsandbox sandbox at
/// creation. Deliberately does NOT inherit the host environment. Additional
/// variables may be added via `sandbox_env_allowlist` (after secret filtering).
pub fn guest_base_env(cfg: &Config) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert(
        "PATH".into(),
        "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".into(),
    );
    env.insert("HOME".into(), "/home/catcode".into());
    env.insert("LANG".into(), "C.UTF-8".into());
    env.insert("LC_ALL".into(), "C.UTF-8".into());
    env.insert("TERM".into(), "dumb".into());
    if std::env::var("CI").is_ok() {
        env.insert("CI".into(), "1".into());
    }
    env.insert("CATCODE_SANDBOX".into(), "1".into());
    env.insert("CATCODE_WORKSPACE".into(), "/workspace".into());
    env.insert("GIT_PAGER".into(), "cat".into());
    env.insert("PAGER".into(), "cat".into());
    // Explicitly-allowlisted host vars (secrets denied even if listed here).
    for name in &cfg.sandbox_env_allowlist {
        if let Ok(val) = std::env::var(name) {
            if !is_secret_var(name) {
                env.insert(name.clone(), val);
            }
        }
    }
    env
}

/// Patterns of environment variables that always carry secrets and must never be
/// forwarded to the guest, even if present in `sandbox_env_allowlist`.
const SECRET_PATTERNS: &[&str] = &[
    "_TOKEN",
    "_SECRET",
    "_PASSWORD",
    "_API_KEY",
    "_CREDENTIAL",
    "_KEY",
];

/// Whether a variable name matches a secret-bearing pattern. Conservative:
/// matches suffixes like `*_TOKEN`, `*_SECRET`, `*_PASSWORD`, `*_API_KEY`, plus
/// well-known cloud/provider/agent-secret names.
pub fn is_secret_var(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    for suf in SECRET_PATTERNS {
        if upper.ends_with(suf) {
            return true;
        }
    }
    matches!(
        upper.as_str(),
        "AWS_ACCESS_KEY_ID"
            | "AWS_SECRET_ACCESS_KEY"
            | "AWS_SESSION_TOKEN"
            | "AZURE_CLIENT_SECRET"
            | "AZURE_TENANT_ID"
            | "AZURE_SUBSCRIPTION_ID"
            | "GOOGLE_APPLICATION_CREDENTIALS"
            | "GOOGLE_CLOUD_PROJECT"
            | "GITHUB_TOKEN"
            | "GH_TOKEN"
            | "NPM_TOKEN"
            | "NPM_AUTHTOKEN"
            | "PYPI_TOKEN"
            | "SSH_AUTH_SOCK"
            | "DOCKER_HOST"
            | "DOCKER_CONFIG"
            | "KUBECONFIG"
    )
}

/// Workspace mount point inside the guest.
pub const GUEST_WORKSPACE: &str = "/workspace";
/// Private guest home directory (never the host home).
pub const GUEST_HOME: &str = "/home/catcode";

/// Translate a workspace-relative path (`""` = workspace root) into the cwd for
/// the active backend. Sandboxed → `/workspace[/rel]`; host →
/// `cfg.workspace[/rel]`. Absolute host paths and `..` escapes are rejected so
/// command input cannot mount/arbitrary-access host paths via the guest.
pub fn effective_cwd(cfg: &Config, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim();
    if rel.is_empty() {
        return Ok(if is_sandbox_enabled() {
            PathBuf::from(GUEST_WORKSPACE)
        } else {
            cfg.workspace.clone()
        });
    }
    // Reject absolute paths and Windows drive letters — they would let a command
    // reach outside the mounted workspace.
    if rel.starts_with('/')
        || rel.starts_with('\\')
        || (rel.len() >= 2 && rel.as_bytes()[1] == b':')
    {
        return Err(format!(
            "path must be workspace-relative, got absolute: {rel:?}"
        ));
    }
    for comp in rel.split(['/', '\\']) {
        if comp == ".." {
            return Err(format!("path must not escape the workspace (..): {rel:?}"));
        }
    }
    if is_sandbox_enabled() {
        Ok(PathBuf::from(GUEST_WORKSPACE).join(rel))
    } else {
        Ok(cfg.workspace.join(rel))
    }
}

/// Resolve a host workspace path to its guest equivalent (used when a caller
/// already holds a canonical host path under the workspace). Returns None if
/// the path is not inside the workspace (host home / `/etc` / …), enforcing
/// confinement even under `Approval::Never`.
pub fn host_to_guest_path(cfg: &Config, host: &Path) -> Option<PathBuf> {
    let ws = cfg
        .workspace
        .canonicalize()
        .ok()
        .unwrap_or_else(|| cfg.workspace.clone());
    let host = host
        .canonicalize()
        .ok()
        .unwrap_or_else(|| host.to_path_buf());
    let rel = host.strip_prefix(&ws).ok()?;
    if rel
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    let mut g = PathBuf::from(GUEST_WORKSPACE);
    g.push(rel);
    Some(g)
}

/// Translate a [`SandboxNetworkMode`] into a human label for status reporting.
pub fn network_label(mode: SandboxNetworkMode) -> &'static str {
    match mode {
        SandboxNetworkMode::None => "none (no network interface)",
        SandboxNetworkMode::Restricted => "restricted",
        SandboxNetworkMode::Allowlist => "allowlist",
    }
}

/// The model-facing description of the `bash` tool for the active shell. When
/// sandboxed, Windows users are told the guest is Linux `bash` (not PowerShell).
pub fn bash_tool_description() -> &'static str {
    if is_sandbox_enabled() {
        return "Run a bash command inside the sandbox microVM (Linux guest). The workspace is mounted at /workspace; stdout+stderr are captured, truncated to 32KB, default 30s timeout. Pass timeout for slow builds. Keep commands short; for complex logic write a script with write_file and run `bash script.sh`. The environment is isolated: host secrets and the host home directory are not available.";
    }
    match effective_shell_kind() {
        ShellKind::Posix => "Run a bash command in the workspace (stdout+stderr, truncated to 32KB, default 30s timeout). Pass timeout for slow builds. Keep commands short; for complex logic write a script with write_file and run bash script.sh.",
        ShellKind::PowerShell => "Run a shell command in the workspace (PowerShell; stdout+stderr, truncated to 32KB, default 30s timeout). Pass timeout for slow builds. Keep commands short; for complex logic write a .ps1 script with write_file and run `powershell -File script.ps1`.",
    }
}

/// Sandbox mode requested by config (without touching the global backend — used
/// during preflight before the backend is initialized).
pub fn config_requests_sandbox(cfg: &Config) -> bool {
    matches!(cfg.sandbox, Sandbox::Microsandbox)
}
