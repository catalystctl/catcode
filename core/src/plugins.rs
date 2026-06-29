// Plugin system: self-bootstrapping hooks loaded from .umans-harness/plugins/.
// Each plugin is a subdirectory with a plugin.json manifest and hook scripts.
// Hooks are spawned as subprocesses with stdin JSON context, stdout JSON response.
// Broken hooks never crash the core; timeouts and parse failures are handled gracefully.
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::Duration;
use tokio::process::Command;
use tokio::io::AsyncWriteExt;

// ---- constants ----

/// Injected into the system prompt so agents can self-bootstrap plugins.
/// Explains the plugin directory layout, manifest format, hook contract,
/// and available hook points. Designed for an LLM to read and act on.
pub const PLUGIN_DOCS: &str = r#"## Plugin System

You can extend the harness with plugins. Plugins are self-contained directories
under `.umans-harness/plugins/`. Each plugin hooks into tool execution and
session lifecycle events to inspect, approve, modify, or log operations.

### Creating a plugin

1. Create a directory: `.umans-harness/plugins/<plugin-name>/`
2. Write a `plugin.json` manifest (see format below)
3. Write executable hook scripts (bash, python, or any language)
4. Make hook scripts executable (`chmod +x hooks/*.sh`)
5. The core loads new plugins on next restart, or you can call the `plugin` tool
   (if loaded by the TUI) to `install`, `remove`, `enable`, or `disable` plugins
   at runtime.

### plugin.json format

```
{
  "name": "my-plugin",
  "version": "0.1.0",
  "description": "What this plugin does",
  "hooks": {
    "pre_write": {
      "script": "hooks/pre_write.sh",
      "timeout_ms": 5000,
      "pass_args": true
    },
    "post_bash": {
      "script": "hooks/post_bash.py",
      "timeout_ms": 30000,
      "pass_args": false
    }
  }
}
```

Fields:
- `name` (required): unique plugin identifier (directory name must match)
- `version` (required): semver string
- `description` (optional): human-readable summary
- `hooks` (optional): map of hook-point name to config
  - `script` (required): path to executable, relative to the plugin directory
  - `timeout_ms` (optional): override the default hook timeout (default: 5s for pre_*, 30s for post_*)
  - `pass_args` (optional): if true, the hook context JSON includes the tool's `args` object (default: false)

### Hook contract

Each hook script receives a single JSON object on stdin and MUST write a single
JSON object to stdout before exiting. Stderr is captured for error reporting.

**Context (stdin → script):**
```
{
  "hook": "pre_write",
  "tool": "write_file",
  "workspace": "/path/to/workspace",
  "args": { "path": "src/file.rs", "content": "..." },
  "session_id": "abc123.jsonl",
  "timestamp": 1719000000
}
```

**Response (script → stdout):**
```
{
  "allow": true,
  "reason": "File passes lint check",
  "modify": { "content": "reformatted code" }
}
```

- `allow` (required, bool): true to proceed, false to block (pre hooks) or skip result (post hooks)
- `reason` (optional, string): human-readable explanation, shown in logs
- `modify` (optional, object): for pre hooks, replaces or augments the original args.
  For pre_write: `{ "content": "reformatted" }` replaces file content.
  For pre_bash: `{ "command": "fixed command" }` replaces the command.
  For pre_read and others: `{ "args": { "path": "new/path", ... } }` replaces full args.
  For post hooks, modify is ignored (the operation already completed).

Safety rules enforced by the core:
- pre_* hooks: non-zero exit, timeout, or JSON parse failure → `allow: false` (blocks the tool)
- post_* hooks: non-zero exit, timeout, or JSON parse failure → silently skipped (tool already ran)
- Disabled plugins are never invoked
- Every hook has a hard timeout (5s default for pre_*, 30s default for post_*)
- Hook failures never crash the core

### Available hook points

| Hook point    | Fires when                              | Type |
|---------------|-----------------------------------------|------|
| pre_bash      | Before a bash command executes          | pre  |
| pre_write     | Before a file write/edit                | pre  |
| pre_read      | Before a file is read                   | pre  |
| post_bash     | After a bash command completes          | post |
| post_write    | After a file write/edit completes       | post |
| post_read     | After a file is read                    | post |
| session_start | When a session begins (prompt received) | lifecycle |
| session_stop  | When a session ends (done/abort)        | lifecycle |
| pre_compact   | Before conversation compaction         | pre  |
| pre_turn      | Before a model request (advisory)      | pre  |

### pre_turn hook (model handoff)

`pre_turn` fires once per assistant turn, after the user message (including any
attached images) is built and before the first model request. It is advisory:
it can remap the model for the turn but can never block it (a missing/broken
hook or `allow:false` is ignored — the turn proceeds with the original model).

Context `args` (set `pass_args: true` in the manifest):
```
{
  "model": "umans-glm-5.2",
  "has_images": true,
  "image_count": 2,
  "models": [ {"id":"...", "vision":true}, ... ]
}
```
Response: return `modify: { "model": "<new-model-id>" }` to swap the turn's
model. The core validates the id against discovered models and emits an `info`
event on handoff. Use this to route image-bearing turns to a vision-capable
model when the active one lacks vision (see the bundled `vision-handoff` plugin).

### Example: a pre_write linter plugin

`.umans-harness/plugins/lint-check/plugin.json`:
```
{
  "name": "lint-check",
  "version": "0.1.0",
  "description": "Run cargo fmt on Rust files before writing",
  "hooks": {
    "pre_write": {
      "script": "hooks/pre_write.sh",
      "timeout_ms": 10000,
      "pass_args": true
    }
  }
}
```

`.umans-harness/plugins/lint-check/hooks/pre_write.sh`:
```bash
#!/bin/bash
input=$(cat)
path=$(echo "$input" | jq -r '.args.path // ""')
content=$(echo "$input" | jq -r '.args.content // ""')

if [[ "$path" == *.rs ]] && command -v rustfmt &>/dev/null; then
  formatted=$(echo "$content" | rustfmt --edition 2021 2>/dev/null)
  if [ $? -eq 0 ] && [ -n "$formatted" ]; then
    jq -n --arg c "$formatted" '{ "allow": true, "reason": "rustfmt applied", "modify": { "content": $c } }'
    exit 0
  fi
fi
echo '{"allow": true}'
```

Remember: `chmod +x .umans-harness/plugins/lint-check/hooks/pre_write.sh`
"#;

/// Valid hook point names. Plugins can register for any of these.
pub const HOOK_POINTS: &[&str] = &[
    "pre_bash",
    "pre_write",
    "pre_read",
    "post_bash",
    "post_write",
    "post_read",
    "session_start",
    "session_stop",
    "pre_compact",
    "pre_turn",
];

/// Default timeout in milliseconds for pre_* hooks (blocking — keep short).
pub const DEFAULT_PRE_TIMEOUT_MS: u64 = 5_000;

/// Default timeout in milliseconds for post_* and lifecycle hooks.
pub const DEFAULT_POST_TIMEOUT_MS: u64 = 30_000;

// ---- manifest deserialization (plugin.json) ----

#[derive(Deserialize, Debug, Clone)]
struct PluginManifest {
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    hooks: HashMap<String, HookManifestEntry>,
}

#[derive(Deserialize, Debug, Clone)]
struct HookManifestEntry {
    script: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    pass_args: bool,
}

// ---- public types ----

/// A loaded plugin with its registered hooks.
#[derive(Clone, Debug)]
pub struct Plugin {
    pub name: String,
    pub version: String,
    pub description: String,
    pub enabled: bool,
    /// Absolute path to the plugin directory on disk.
    pub source_path: PathBuf,
    /// Hook name → config map.
    pub hooks: HashMap<String, HookConfig>,
}

/// Configuration for one hook within a plugin.
#[derive(Clone, Debug)]
pub struct HookConfig {
    /// Absolute path to the executable hook script.
    pub script: PathBuf,
    /// Hard timeout in milliseconds for this hook.
    pub timeout_ms: u64,
    /// Whether to include tool args in the hook context JSON.
    pub pass_args: bool,
}

/// Result returned from executing a hook.
#[derive(Clone, Debug)]
pub struct HookResult {
    /// Whether the operation is allowed to proceed.
    pub allow: bool,
    /// Human-readable explanation from the hook.
    pub reason: String,
    /// Optional modified arguments (pre hooks only; ignored for post hooks).
    pub modify: Option<Value>,
}

// ---- PluginManager ----

/// Manages the lifecycle of all installed plugins.
/// Holds an in-memory registry behind a `RwLock`.
pub struct PluginManager {
    plugins_dir: PathBuf,
    plugins: RwLock<HashMap<String, Plugin>>,
}

impl PluginManager {
    /// Create a new manager and scan/load all plugins from `plugins_dir`.
    /// The directory is created if it does not exist.
    pub fn new(plugins_dir: PathBuf) -> Self {
        let mgr = PluginManager {
            plugins_dir,
            plugins: RwLock::new(HashMap::new()),
        };
        mgr.scan_and_load();
        mgr
    }

    /// Re-scan the plugins directory and load/reload all valid plugins.
    /// Invalid plugins are skipped with a log message to stderr but never crash.
    fn scan_and_load(&self) {
        let _ = std::fs::create_dir_all(&self.plugins_dir);
        let mut plugins = self.plugins.write().unwrap();
        plugins.clear();

        let rd = match std::fs::read_dir(&self.plugins_dir) {
            Ok(r) => r,
            Err(_) => return,
        };

        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("plugin.json");
            if !manifest_path.exists() {
                continue;
            }
            match Self::load_plugin_from_dir(&path) {
                Ok(plugin) => {
                    plugins.insert(plugin.name.clone(), plugin);
                }
                Err(e) => {
                    eprintln!(
                        "[plugins] failed to load plugin in {:?}: {e}",
                        path.file_name().unwrap_or_default()
                    );
                }
            }
        }
    }

    /// Load a single plugin from a directory containing plugin.json.
    fn load_plugin_from_dir(dir: &Path) -> Result<Plugin, String> {
        let manifest_path = dir.join("plugin.json");
        let raw = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("cannot read plugin.json: {e}"))?;

        let manifest: PluginManifest = serde_json::from_str(&raw)
            .map_err(|e| format!("plugin.json parse error: {e}"))?;

        if manifest.name.is_empty() {
            return Err("plugin name is empty".into());
        }

        // Canonicalize the plugin directory for path-confinement checks.
        let canon_dir = std::fs::canonicalize(dir)
            .unwrap_or_else(|_| dir.to_path_buf());

        let mut hooks: HashMap<String, HookConfig> = HashMap::new();
        for (hook_name, entry) in &manifest.hooks {
            if !HOOK_POINTS.contains(&hook_name.as_str()) {
                eprintln!(
                    "[plugins] unknown hook point '{}' in plugin '{}'; skipping",
                    hook_name, manifest.name
                );
                continue;
            }

            let script_rel = Path::new(&entry.script);

            // Reject `..` escapes in the relative path before any join.
            {
                use std::path::Component;
                for comp in script_rel.components() {
                    if let Component::ParentDir = comp {
                        return Err(format!(
                            "hook script {:?} escapes the plugin directory",
                            entry.script
                        ));
                    }
                }
            }

            let script_abs = canon_dir.join(script_rel);

            // Canonicalize if possible to catch symlink escapes.
            let canon_script = std::fs::canonicalize(&script_abs)
                .unwrap_or_else(|_| script_abs.clone());
            if !canon_script.starts_with(&canon_dir) {
                return Err(format!(
                    "hook script {:?} escapes the plugin directory",
                    entry.script
                ));
            }

            if !canon_script.exists() {
                return Err(format!("hook script {:?} does not exist", entry.script));
            }

            // Cross-platform executable check (Unix permission bit, or
            // extension/presence on Windows where there is no exec bit).
            let is_exe = is_executable(&canon_script);
            if !is_exe {
                return Err(format!(
                    "hook script {:?} is not executable (try chmod +x)",
                    entry.script
                ));
            }

            let timeout_ms = entry
                .timeout_ms
                .unwrap_or_else(|| default_hook_timeout(hook_name));

            hooks.insert(
                hook_name.clone(),
                HookConfig {
                    script: canon_script,
                    timeout_ms,
                    pass_args: entry.pass_args,
                },
            );
        }

        Ok(Plugin {
            name: manifest.name,
            version: manifest.version,
            description: manifest.description,
            enabled: true,
            source_path: canon_dir,
            hooks,
        })
    }

    /// Install a plugin from `source_path` (a directory containing plugin.json).
    /// The plugin directory is copied into the managed plugins directory and
    /// registered. Returns an error if a plugin with the same name already exists
    /// or if validation fails.
    pub fn install(&self, source_path: &Path) -> Result<Plugin, String> {
        let source = if source_path.is_absolute() {
            source_path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(source_path)
        };
        if !source.is_dir() {
            return Err(format!("{:?} is not a directory", source_path));
        }
        let manifest_path = source.join("plugin.json");
        if !manifest_path.exists() {
            return Err(format!("no plugin.json found in {:?}", source_path));
        }

        // Pre-validate the plugin from source before copying.
        let plugin = Self::load_plugin_from_dir(&source)?;

        // Check for name collision.
        {
            let plugins = self.plugins.read().unwrap();
            if plugins.contains_key(&plugin.name) {
                return Err(format!(
                    "plugin '{}' is already installed; remove it first or use a different name",
                    plugin.name
                ));
            }
        }

        let dest_dir = self.plugins_dir.join(&plugin.name);
        if dest_dir.exists() {
            let _ = std::fs::remove_dir_all(&dest_dir);
        }

        copy_dir(&source, &dest_dir)?;

        // Re-load from the copied location so paths point to the managed dir.
        let installed = Self::load_plugin_from_dir(&dest_dir)?;

        self.plugins
            .write()
            .unwrap()
            .insert(installed.name.clone(), installed.clone());

        Ok(installed)
    }

    /// Remove a plugin by name. Deletes the plugin directory from disk and
    /// unregisters it from the in-memory registry.
    pub fn remove(&self, name: &str) -> Result<(), String> {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(plugin) = plugins.remove(name) {
            let _ = std::fs::remove_dir_all(&plugin.source_path);
            Ok(())
        } else {
            Err(format!("plugin '{}' not found", name))
        }
    }

    /// Enable a previously-disabled plugin by name.
    pub fn enable(&self, name: &str) -> Result<(), String> {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(plugin) = plugins.get_mut(name) {
            plugin.enabled = true;
            Ok(())
        } else {
            Err(format!("plugin '{}' not found", name))
        }
    }

    /// Disable a plugin by name (keeps it on disk, stops invoking hooks).
    pub fn disable(&self, name: &str) -> Result<(), String> {
        let mut plugins = self.plugins.write().unwrap();
        if let Some(plugin) = plugins.get_mut(name) {
            plugin.enabled = false;
            Ok(())
        } else {
            Err(format!("plugin '{}' not found", name))
        }
    }

    /// Return a snapshot of all registered plugins (name → Plugin).
    pub fn list(&self) -> HashMap<String, Plugin> {
        self.plugins.read().unwrap().clone()
    }

    /// Get all enabled hook configs for a given hook point name.
    /// Returns a vec of (plugin_name, HookConfig) pairs so the caller can
    /// iterate and merge results.
    pub fn get_hook_configs(&self, hook_name: &str) -> Vec<(String, HookConfig)> {
        self.plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.enabled)
            .filter_map(|p| p.hooks.get(hook_name).map(|c| (p.name.clone(), c.clone())))
            .collect()
    }

    /// Look up a single plugin by name.
    pub fn get_plugin(&self, name: &str) -> Option<Plugin> {
        self.plugins.read().unwrap().get(name).cloned()
    }
}

// ---- hook execution ----

/// Execute a single hook script and return its result.
///
/// The hook receives `context` JSON on stdin. It must write a JSON response
/// (see PLUGIN_DOCS for schema) to stdout. The function handles timeouts,
/// non-zero exits, and parse failures according to the safety rules:
///
/// - **pre_* hooks**: non-zero exit, timeout, or parse failure → deny
/// - **post_* / lifecycle hooks**: non-zero exit, timeout, or parse failure → skip
///
/// The `hook_name` prefix ("pre_" vs "post_" etc.) determines the safety rule.
/// Disabled plugin checks are handled before calling this function.
pub async fn execute_hook(
    hook_name: &str,
    plugin_name: &str,
    config: &HookConfig,
    context: &Value,
) -> HookResult {
    let is_pre = hook_name.starts_with("pre_");

    let deny = |reason: String| HookResult {
        allow: false,
        reason,
        modify: None,
    };

    let skip = |reason: String| HookResult {
        allow: true,
        reason: format!("[{plugin_name}] {reason}"),
        modify: None,
    };

    // Spawn the hook script.
    let script_path = &config.script;
    let mut child = match hook_command(script_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("failed to spawn hook script {:?}: {e}", script_path);
            return if is_pre { deny(msg) } else { skip(msg) };
        }
    };

    // Write the context JSON to stdin, then close it so the script can proceed.
    let context_bytes = serde_json::to_vec(context).unwrap_or_default();
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&context_bytes).await;
        let _ = stdin.shutdown().await;
    }

    let timeout_dur = Duration::from_millis(config.timeout_ms);
    let output_result = tokio::time::timeout(timeout_dur, child.wait_with_output()).await;

    match output_result {
        Ok(Ok(output)) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let msg = format!(
                    "hook '{}' exited with {}: {}",
                    hook_name,
                    output.status,
                    stderr.trim()
                );
                return if is_pre { deny(msg) } else { skip(msg) };
            }

            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if stdout.is_empty() {
                let msg = format!("hook '{}' returned empty stdout", hook_name);
                return if is_pre { deny(msg) } else { skip(msg) };
            }

            let response: Value = match serde_json::from_str(&stdout) {
                Ok(v) => v,
                Err(e) => {
                    let msg = format!(
                        "hook '{}' returned invalid JSON: {e}",
                        hook_name
                    );
                    return if is_pre { deny(msg) } else { skip(msg) };
                }
            };

            let allow = response
                .get("allow")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let reason = response
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let modify = response.get("modify").cloned();

            // For post hooks, we never block — "allow: false" just means
            // the hook observed an issue, it doesn't roll back the operation.
            if !is_pre && !allow {
                return HookResult {
                    allow: true,
                    reason: format!("[{plugin_name}] {reason}"),
                    modify: None,
                };
            }

            HookResult {
                allow,
                reason,
                modify,
            }
        }
        Ok(Err(e)) => {
            let msg = format!("hook '{}' wait error: {e}", hook_name);
            if is_pre { deny(msg) } else { skip(msg) }
        }
        Err(_elapsed) => {
            let msg = format!(
                "hook '{}' timed out after {}ms",
                hook_name, config.timeout_ms
            );
            if is_pre { deny(msg) } else { skip(msg) }
        }
    }
}

/// Build the standard hook context JSON object.
///
/// The caller supplies the hook point name, tool name (empty string for
/// lifecycle hooks), workspace path, optional tool args, and session id.
/// If `pass_args` is false, the `args` field is omitted from the context.
pub fn build_context(
    hook_name: &str,
    tool_name: &str,
    workspace: &str,
    args: Option<&Value>,
    session_id: &str,
    pass_args: bool,
) -> Value {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut ctx = serde_json::json!({
        "hook": hook_name,
        "tool": tool_name,
        "workspace": workspace,
        "session_id": session_id,
        "timestamp": timestamp,
    });

    if pass_args {
        if let Some(a) = args {
            if let Some(obj) = ctx.as_object_mut() {
                obj.insert("args".to_string(), a.clone());
            }
        }
    }

    ctx
}

// ---- helpers ----

/// Return the default timeout for a hook point.
fn default_hook_timeout(hook_name: &str) -> u64 {
    if hook_name.starts_with("pre_") {
        DEFAULT_PRE_TIMEOUT_MS
    } else {
        DEFAULT_POST_TIMEOUT_MS
    }
}

/// Cross-platform check for whether a hook script is executable.
/// - Unix: any executable permission bit set (owner/group/other).
/// - Windows / non-Unix: no permission bit exists, so any file that exists
///   counts as executable (the OS governs launch by extension; a bad or
///   missing interpreter surfaces as a spawn error at hook execution time).
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}

/// Build the command to run a hook script, selecting the right interpreter by
/// extension so plugins work cross-platform. On Unix a shebang handles `*.sh`;
/// on Windows `.bat`/`.cmd`/`.exe` launch directly, `.ps1` uses powershell,
/// `.py` uses python, and `.sh`/`.bash` use `bash` (Git Bash/WSL) when present.
/// `UMANS_HARNESS_SHELL` overrides the interpreter for `.sh`/`.bash`.
fn hook_command(script: &Path) -> Command {
    let ext = script
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "bat" | "cmd" | "exe" | "com" => Command::new(script),
        "ps1" => {
            let mut c = Command::new("powershell");
            c.arg("-NoProfile")
                .arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-File")
                .arg(script);
            c
        }
        "py" => {
            let mut c = Command::new("python");
            c.arg(script);
            c
        }
        "sh" | "bash" => {
            // Prefer an explicit override, then bash (Git Bash/WSL on Windows).
            // On bare Windows without bash the spawn fails → graceful pre-hook deny.
            if let Ok(shell) = std::env::var("UMANS_HARNESS_SHELL") {
                let mut c = Command::new(shell);
                c.arg(script);
                c
            } else {
                let mut c = Command::new("bash");
                c.arg(script);
                c
            }
        }
        _ => Command::new(script),
    }
}

/// Recursively copy a directory from `src` to `dst`.
fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst)
        .map_err(|e| format!("mkdir {:?}: {e}", dst))?;

    let rd = std::fs::read_dir(src)
        .map_err(|e| format!("read_dir {:?}: {e}", src))?;

    for entry in rd {
        let entry = entry.map_err(|e| format!("dir entry error: {e}"))?;
        let ft = entry
            .file_type()
            .map_err(|e| format!("file_type error: {e}"))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if ft.is_dir() {
            copy_dir(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("copy {:?} -> {:?}: {e}", src_path, dst_path))?;
        }
    }
    Ok(())
}

// ---- tests ----

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }
    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {
        // No executable bit on Windows; hooks launch by extension.
    }

    /// Create a temporary directory that is cleaned up on drop.
    struct TmpDir {
        path: PathBuf,
    }

    impl TmpDir {
        fn new(prefix: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static N: AtomicU64 = AtomicU64::new(0);
            let n = N.fetch_add(1, Ordering::SeqCst);
            let path =
                std::env::temp_dir().join(format!("umans_harness_plugin_test_{}_{}", prefix, n));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TmpDir { path }
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// Write a minimal executable shell script that outputs the given JSON.
    fn write_hook_script(dir: &Path, name: &str, stdout_json: &str, exit_code: u32) -> PathBuf {
        let script = dir.join(name);
        let content = format!(
            "#!/bin/sh\necho '{}'\nexit {}\n",
            stdout_json, exit_code
        );
        fs::write(&script, &content).unwrap();
        make_executable(&script);
        script
    }

    /// Write a complete plugin to a directory.
    fn write_plugin(dir: &Path, name: &str, version: &str, hooks_json: &str) {
        let manifest = format!(
            r#"{{
  "name": "{}",
  "version": "{}",
  "description": "Test plugin",
  "hooks": {}
}}"#,
            name, version, hooks_json
        );
        fs::write(dir.join("plugin.json"), manifest).unwrap();
    }

    // ---- manifest loading ----

    #[test]
    fn load_minimal_plugin() {
        let tmp = TmpDir::new("load_minimal");
        write_plugin(&tmp.path, "minimal", "1.0.0", "{}");
        let plugin = PluginManager::load_plugin_from_dir(&tmp.path).unwrap();
        assert_eq!(plugin.name, "minimal");
        assert_eq!(plugin.version, "1.0.0");
        assert_eq!(plugin.hooks.len(), 0);
        assert!(plugin.enabled);
    }

    #[test]
    fn load_plugin_with_hooks() {
        let tmp = TmpDir::new("load_with_hooks");
        let hooks_dir = tmp.path.join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();
        let pre_script = write_hook_script(&hooks_dir, "pre_write.sh", r#"{"allow":true}"#, 0);
        let post_script = write_hook_script(&hooks_dir, "post_bash.sh", r#"{"allow":true}"#, 0);

        // Use relative paths in the manifest.
        write_plugin(
            &tmp.path,
            "with-hooks",
            "0.2.0",
            &format!(
                r#"{{
          "pre_write": {{ "script": "hooks/pre_write.sh", "timeout_ms": 7000, "pass_args": true }},
          "post_bash": {{ "script": "hooks/post_bash.sh" }}
        }}"#
            ),
        );

        let plugin = PluginManager::load_plugin_from_dir(&tmp.path).unwrap();
        assert_eq!(plugin.hooks.len(), 2);

        let pre = plugin.hooks.get("pre_write").unwrap();
        assert_eq!(pre.script, std::fs::canonicalize(&pre_script).unwrap());
        assert_eq!(pre.timeout_ms, 7000);
        assert!(pre.pass_args);

        let post = plugin.hooks.get("post_bash").unwrap();
        assert_eq!(post.script, std::fs::canonicalize(&post_script).unwrap());
        assert_eq!(post.timeout_ms, DEFAULT_POST_TIMEOUT_MS);
        assert!(!post.pass_args);
    }

    #[test]
    fn load_rejects_missing_script() {
        let tmp = TmpDir::new("load_missing_script");
        write_plugin(
            &tmp.path,
            "bad",
            "1.0.0",
            r#"{"pre_write": {"script": "hooks/nonexistent.sh"}}"#,
        );
        let result = PluginManager::load_plugin_from_dir(&tmp.path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn load_rejects_non_executable() {
        let tmp = TmpDir::new("load_not_exe");
        let hooks_dir = tmp.path.join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();
        let script = hooks_dir.join("hook.sh");
        fs::write(&script, "#!/bin/sh\necho ok\n").unwrap();
        // Leave without +x.
        write_plugin(
            &tmp.path,
            "not-exe",
            "1.0.0",
            r#"{"pre_write": {"script": "hooks/hook.sh"}}"#,
        );
        let result = PluginManager::load_plugin_from_dir(&tmp.path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not executable"));
    }

    #[test]
    fn load_rejects_script_escape() {
        let tmp = TmpDir::new("load_escape");
        write_plugin(
            &tmp.path,
            "escape-artist",
            "1.0.0",
            r#"{"pre_write": {"script": "../hooks/outside.sh"}}"#,
        );
        let result = PluginManager::load_plugin_from_dir(&tmp.path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("escapes"));
    }

    #[test]
    fn load_skips_unknown_hook() {
        let tmp = TmpDir::new("load_unknown_hook");
        let hooks_dir = tmp.path.join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();
        write_hook_script(&hooks_dir, "hook.sh", r#"{"allow":true}"#, 0);
        write_plugin(
            &tmp.path,
            "weird",
            "1.0.0",
            r#"{"pre_launch_missiles": {"script": "hooks/hook.sh"}}"#,
        );
        let plugin = PluginManager::load_plugin_from_dir(&tmp.path).unwrap();
        assert!(plugin.hooks.is_empty()); // unknown hook skipped
    }

    #[test]
    fn load_rejects_bad_json() {
        let tmp = TmpDir::new("load_bad_json");
        fs::write(tmp.path.join("plugin.json"), "not valid {{{").unwrap();
        let result = PluginManager::load_plugin_from_dir(&tmp.path);
        assert!(result.is_err());
    }

    #[test]
    fn load_rejects_empty_name() {
        let tmp = TmpDir::new("load_empty_name");
        write_plugin(&tmp.path, "", "1.0.0", "{}");
        let result = PluginManager::load_plugin_from_dir(&tmp.path);
        assert!(result.is_err());
    }

    // ---- PluginManager lifecycle ----

    #[test]
    fn manager_loads_plugins_on_new() {
        let tmp = TmpDir::new("mgr_loads");
        let plugin_dir = tmp.path.join("test-plugin");
        fs::create_dir_all(&plugin_dir.join("hooks")).unwrap();
        write_hook_script(
            &plugin_dir.join("hooks"),
            "hook.sh",
            r#"{"allow":true}"#,
            0,
        );
        write_plugin(
            &plugin_dir,
            "test-plugin",
            "1.0.0",
            r#"{"pre_write": {"script": "hooks/hook.sh"}}"#,
        );

        let mgr = PluginManager::new(tmp.path.clone());
        let plugins = mgr.list();
        assert_eq!(plugins.len(), 1);
        assert!(plugins.contains_key("test-plugin"));
    }

    #[test]
    fn install_and_remove_plugin() {
        let tmp = TmpDir::new("mgr_install");
        let mgr = PluginManager::new(tmp.path.join("managed"));

        // Create a plugin source dir.
        let src = TmpDir::new("install_src");
        fs::create_dir_all(src.path.join("hooks")).unwrap();
        write_hook_script(
            &src.path.join("hooks"),
            "hook.sh",
            r#"{"allow":true}"#,
            0,
        );
        write_plugin(
            &src.path,
            "fresh",
            "2.0.0",
            r#"{"post_write": {"script": "hooks/hook.sh"}}"#,
        );

        let installed = mgr.install(&src.path).unwrap();
        assert_eq!(installed.name, "fresh");
        assert_eq!(installed.version, "2.0.0");

        // Check that it was copied into the managed dir.
        assert!(mgr.list().contains_key("fresh"));
        assert!(tmp.path.join("managed/fresh/plugin.json").exists());

        // Remove it.
        mgr.remove("fresh").unwrap();
        assert!(mgr.list().is_empty());
        assert!(!tmp.path.join("managed/fresh").exists());
    }

    #[test]
    fn install_rejects_duplicate() {
        let tmp = TmpDir::new("mgr_dup");
        let mgr = PluginManager::new(tmp.path.join("managed"));

        let src = TmpDir::new("dup_src");
        fs::create_dir_all(src.path.join("hooks")).unwrap();
        write_hook_script(
            &src.path.join("hooks"),
            "h.sh",
            r#"{"allow":true}"#,
            0,
        );
        write_plugin(
            &src.path,
            "dup",
            "1.0.0",
            r#"{"pre_read": {"script": "hooks/h.sh"}}"#,
        );

        mgr.install(&src.path).unwrap();
        let result = mgr.install(&src.path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already installed"));
    }

    #[test]
    fn enable_disable_toggle() {
        let tmp = TmpDir::new("mgr_toggle");
        let mgr = PluginManager::new(tmp.path.join("managed"));

        let src = TmpDir::new("toggle_src");
        fs::create_dir_all(src.path.join("hooks")).unwrap();
        write_hook_script(
            &src.path.join("hooks"),
            "h.sh",
            r#"{"allow":true}"#,
            0,
        );
        write_plugin(
            &src.path,
            "toggle-me",
            "1.0.0",
            r#"{"pre_write": {"script": "hooks/h.sh"}}"#,
        );

        mgr.install(&src.path).unwrap();

        // Initially enabled.
        assert!(mgr.get_plugin("toggle-me").unwrap().enabled);

        mgr.disable("toggle-me").unwrap();
        assert!(!mgr.get_plugin("toggle-me").unwrap().enabled);

        mgr.enable("toggle-me").unwrap();
        assert!(mgr.get_plugin("toggle-me").unwrap().enabled);

        // Disabled plugins are excluded from hook configs.
        mgr.disable("toggle-me").unwrap();
        let configs = mgr.get_hook_configs("pre_write");
        assert!(configs.is_empty());
    }

    #[test]
    fn enable_disable_unknown_is_error() {
        let tmp = TmpDir::new("mgr_unknown");
        let mgr = PluginManager::new(tmp.path.clone());
        assert!(mgr.enable("nope").is_err());
        assert!(mgr.disable("nope").is_err());
        assert!(mgr.remove("nope").is_err());
    }

    // ---- execute_hook ----

    #[tokio::test]
    async fn execute_hook_allow() {
        let tmp = TmpDir::new("exec_allow");
        let script = write_hook_script(
            &tmp.path,
            "allow.sh",
            r#"{"allow": true, "reason": "all good"}"#,
            0,
        );
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("pre_write", "write_file", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("pre_write", "test-plugin", &config, &ctx).await;
        assert!(result.allow);
        assert_eq!(result.reason, "all good");
        assert!(result.modify.is_none());
    }

    #[tokio::test]
    async fn execute_hook_deny() {
        let tmp = TmpDir::new("exec_deny");
        let script = write_hook_script(
            &tmp.path,
            "deny.sh",
            r#"{"allow": false, "reason": "blocked"}"#,
            0,
        );
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("pre_write", "write_file", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("pre_write", "test-plugin", &config, &ctx).await;
        assert!(!result.allow);
        assert_eq!(result.reason, "blocked");
    }

    #[tokio::test]
    async fn execute_hook_with_modify() {
        let tmp = TmpDir::new("exec_modify");
        let response = r#"{"allow": true, "reason": "reformatted", "modify": {"content": "new"}}"#;
        let script = write_hook_script(&tmp.path, "modify.sh", response, 0);
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("pre_write", "write_file", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("pre_write", "test-plugin", &config, &ctx).await;
        assert!(result.allow);
        assert_eq!(result.reason, "reformatted");
        assert_eq!(result.modify, Some(json!({"content": "new"})));
    }

    #[tokio::test]
    async fn execute_hook_nonzero_exit_pre_denies() {
        let tmp = TmpDir::new("exec_exit_pre");
        let script = write_hook_script(
            &tmp.path,
            "fail.sh",
            r#"{"allow": true}"#,
            1, // exits with 1
        );
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("pre_write", "write_file", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("pre_write", "test-plugin", &config, &ctx).await;
        assert!(!result.allow);
        assert!(result.reason.contains("exited"));
    }

    #[tokio::test]
    async fn execute_hook_nonzero_exit_post_skips() {
        let tmp = TmpDir::new("exec_exit_post");
        let script = write_hook_script(
            &tmp.path,
            "fail.sh",
            r#"{"allow": true}"#,
            1,
        );
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("post_bash", "bash", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("post_bash", "test-plugin", &config, &ctx).await;
        // post hooks: non-zero exit is skipped, operation continues.
        assert!(result.allow);
        assert!(result.reason.contains("exited"));
        assert!(result.modify.is_none());
    }

    #[tokio::test]
    async fn execute_hook_bad_json_pre_denies() {
        let tmp = TmpDir::new("exec_bad_json");
        let script = write_hook_script(&tmp.path, "bad.sh", "NOT JSON", 0);
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("pre_write", "write_file", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("pre_write", "test-plugin", &config, &ctx).await;
        assert!(!result.allow);
        assert!(result.reason.contains("invalid JSON"));
    }

    #[tokio::test]
    async fn execute_hook_timeout_pre_denies() {
        let tmp = TmpDir::new("exec_timeout");
        // Script sleeps long enough to trigger the timeout.
        let script = tmp.path.join("sleep.sh");
        fs::write(
            &script,
            "#!/bin/sh\nsleep 10\necho '{\"allow\":true}'\n",
        )
        .unwrap();
        make_executable(&script);

        let config = HookConfig {
            script,
            timeout_ms: 200, // very short timeout
            pass_args: false,
        };
        let ctx = build_context("pre_write", "write_file", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("pre_write", "test-plugin", &config, &ctx).await;
        assert!(!result.allow);
        assert!(result.reason.contains("timed out"));
    }

    #[tokio::test]
    async fn execute_hook_post_always_allows_even_on_deny() {
        // For post hooks, even if the hook returns allow:false, we don't block.
        let tmp = TmpDir::new("exec_post_deny");
        let script = write_hook_script(
            &tmp.path,
            "deny.sh",
            r#"{"allow": false, "reason": "saw an issue"}"#,
            0,
        );
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("post_write", "write_file", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("post_write", "test-plugin", &config, &ctx).await;
        assert!(result.allow);
        assert_eq!(result.modify, None);
    }

    #[tokio::test]
    async fn execute_hook_empty_stdout_pre_denies() {
        let tmp = TmpDir::new("exec_empty");
        let script = write_hook_script(&tmp.path, "empty.sh", "", 0);
        let config = HookConfig {
            script,
            timeout_ms: 5000,
            pass_args: false,
        };
        let ctx = build_context("pre_bash", "bash", "/ws", None, "sess.jsonl", false);
        let result = execute_hook("pre_bash", "test-plugin", &config, &ctx).await;
        assert!(!result.allow);
        assert!(result.reason.contains("empty stdout"));
    }

    // ---- build_context ----

    #[test]
    fn build_context_structure() {
        let ctx = build_context(
            "pre_write",
            "write_file",
            "/home/user/project",
            Some(&json!({"path": "src/main.rs", "content": "fn main() {}"})),
            "session_123.jsonl",
            true,
        );

        assert_eq!(ctx["hook"], "pre_write");
        assert_eq!(ctx["tool"], "write_file");
        assert_eq!(ctx["workspace"], "/home/user/project");
        assert_eq!(ctx["session_id"], "session_123.jsonl");
        assert!(ctx["timestamp"].as_u64().is_some());
        assert_eq!(ctx["args"]["path"], "src/main.rs");
        assert_eq!(ctx["args"]["content"], "fn main() {}");
    }

    #[test]
    fn build_context_omits_args_when_pass_args_false() {
        let ctx = build_context(
            "pre_write",
            "write_file",
            "/ws",
            Some(&json!({"secret": "value"})),
            "sess.jsonl",
            false,
        );
        assert!(ctx.get("args").is_none());
    }

    #[test]
    fn build_context_handles_none_args() {
        let ctx = build_context(
            "session_start",
            "",
            "/ws",
            None,
            "sess.jsonl",
            true,
        );
        assert!(ctx.get("args").is_none());
    }

    // ---- default timeouts ----

    #[test]
    fn pre_hooks_get_short_timeout() {
        assert_eq!(default_hook_timeout("pre_bash"), 5_000);
        assert_eq!(default_hook_timeout("pre_write"), 5_000);
        assert_eq!(default_hook_timeout("pre_read"), 5_000);
        assert_eq!(default_hook_timeout("pre_compact"), 5_000);
    }

    #[test]
    fn post_hooks_get_long_timeout() {
        assert_eq!(default_hook_timeout("post_bash"), 30_000);
        assert_eq!(default_hook_timeout("post_write"), 30_000);
        assert_eq!(default_hook_timeout("session_start"), 30_000);
        assert_eq!(default_hook_timeout("session_stop"), 30_000);
    }
}
