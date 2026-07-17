# Troubleshooting

Common problems when installing, configuring, running, or using Catalyst Code,
organized by symptom.

---

## Installation Issues

### `curl | bash` fails with "Permission denied"

**Likely causes:** The install script tries to write to `/usr/local/bin/` or
`/opt/catalyst-code/` without the required permissions.

**Check:**
- Run with `--dry-run` first to see the planned paths:
  ```bash
  curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.sh | bash -s -- --dry-run
  ```
- Check whether `/usr/local/bin` is writable.

**Fix:**
- Install with `--prefix` to use a user-local directory:
  ```bash
  curl -fsSL ... | bash -s -- --prefix ~/.local/bin
  ```
- Or run as root (not recommended — prefer the prefix approach).

### `catcode: command not found` after install

**Likely cause:** The install prefix is not in `PATH`.

**Check:**
- Run `which catcode` or `catcode --version`.
- Check the prefix used during installation (default: `/usr/local/bin`).

**Fix:**
- Add the install prefix to your `PATH`:
  ```bash
  export PATH="/usr/local/bin:$PATH"
  ```
  Add the line to `~/.bashrc`, `~/.zshrc`, or equivalent.

### Windows: `iex` closes my shell

**Likely cause:** `iex` (Invoke-Expression) uses the caller's shell process.
After `| iex`, the shell exits because the installer runs `exit`.

**Fix:**
- Open a **new** PowerShell window after installation so `PATH` reloads.
- Or use the scriptblock form to avoid the `iex` exit issue:
  ```powershell
  & ([scriptblock]::Create((irm https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.ps1)))
  ```

### Windows installer fails with missing dependencies

**Likely cause:** The web frontend requires Node.js or Bun for runtime
execution, or the MSI installer requires Windows Installer service.

**Fix:**
- Install Node.js or Bun first (the web bundle is prebuilt; only the runtime
  is needed).
- For the standalone `.exe`, no runtime is needed — download the
  `catcode-<ver>-windows-x86_64.exe` from [releases](https://github.com/catalystctl/catcode/releases).

---

## Core Won't Start

### "core: command not found" or binary missing

**Likely causes:** The core binary was not installed, was installed to a
different prefix, or the install script's binary download failed.

**Check:**
- Run `catcode --version`. If the TUI starts but the core doesn't, look for
  the core binary: `which catcode-core` or check the install prefix.
- On Linux: `/usr/local/bin/catcode-core`
- On macOS: `/usr/local/bin/catcode-core` or `~/Library/Application Support/...`
- On Windows: in the install directory (same as `catcode.exe`).

**Fix:**
- Re-run the installer. Use `--dry-run` first to confirm download URLs.
- If downloading from a mirror, check `--base-url`.
- Build from source if binaries are unavailable for your platform:
  ```bash
  curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.sh | bash -s -- --build-from-source
  ```

### Core crashes immediately on launch

**Likely causes:** Missing model provider configuration, corrupt session file,
or incompatible system libraries.

**Check:**
- Run the TUI with verbose output to see the core's stderr.
- Look for a core crash log in the session directory.
- Try starting without a session file:
  ```bash
  catcode --session /tmp/test-session.jsonl
  ```
- Check `~/.config/catalyst-code/config.json` for syntax errors.

**Fix:**
- Delete or rename the existing session file to rule out corruption.
- Reset the config: `mv ~/.config/catalyst-code/config.json ~/.config/catalyst-code/config.json.bak`
- Ensure at least one provider is configured (`/login` in the TUI).

### Port conflict (web frontend)

**Symptom:** The web UI says "port already in use" or the service won't start.

**Likely cause:** Another process is already using the default port `49283`.

**Check:**
```bash
lsof -i :49283
```
Or on Windows:
```powershell
netstat -ano | findstr :49283
```

**Fix:**
- Start the service on a different port:
  ```bash
  catcode --port 8080
  ```
- Or pass the port during install:
  ```bash
  curl -fsSL ... | bash -s -- --port 8080
  ```
- Kill the existing process if it's a stale Catalyst Code instance.

---

## Model Connectivity

### "No models available" or provider not listed in `/login`

**Likely cause:** The provider configuration is missing, incomplete, or the
base URL is unreachable.

**Check:**
- Run `/login` to see configured providers and their status.
- Check `~/.config/catalyst-code/config.json` for the provider entry.
- Verify the `base_url` is reachable: `curl -I <base_url>/models`

**Fix:**
- Add a key-based provider:
  ```bash
  catcode --add-provider my-provider --provider-kind openai --base-url https://api.example.com/v1
  ```
- Set the API key environment variable: `export MY_PROVIDER_API_KEY=sk-...`
- For Umans or OpenRouter, use the built-in login flow (`/login`).

### "Invalid API key" or authentication failure (HTTP 401)

**Symptom:** The model responds with "401 Unauthorized" or "Invalid API key"
immediately after the first request.

**Likely cause:** The API key is wrong, expired, or the environment variable
is not set.

**Check:**
- Verify the env var name matches the provider's `api_key_env` in config.json.
  ```bash
  echo $YOUR_PROVIDER_API_KEY
  ```
- Check the provider config:
  ```bash
  cat ~/.config/catalyst-code/config.json | grep -A 10 '"providers"'
  ```

**Fix:**
- Set the correct env var: `export YOUR_PROVIDER_API_KEY=<actual-key>`
- If the key was set inline in the config (not recommended), move it to an
  env var.
- Restart the TUI after setting the env var (the config is read at startup).

### "User not found" (HTTP 401) on OpenRouter

**Symptom:** OpenRouter returns "User not found" with a 401 status.

**Likely cause:** Transient propagation delay after creating a new API key or
account. OpenRouter's key validation can lag by several seconds.

**Check:**
- Wait 10–30 seconds and retry.
- Verify the key is valid at https://openrouter.ai/keys.

**Fix:**
- Retry the request. The error is almost always transient.
- If it persists beyond 60 seconds, generate a new API key at OpenRouter.
- The provider module has retry/backoff that handles this automatically in
  most cases.

### Rate-limited (HTTP 429)

**Symptom:** The model returns "429 Too Many Requests" or provider rate-limit
errors after a burst of requests.

**Check:**
- Run `/usage` to see provider plan and rate-limit windows.
- Check the provider's rate-limit headers in verbose mode.

**Fix:**
- Wait for the rate-limit window to reset. The provider module has an automatic
  cooldown mechanism (default 30s minimum for 429s).
- Upgrade your provider plan for higher rate limits.
- Reduce `parallel_concurrency` in subagent configuration to avoid burst
  requests.

### "Connection refused" or timeout

**Symptom:** The core cannot reach the model provider's API endpoint.

**Likely cause:** Network issues, incorrect `base_url`, or a local model server
that is not running.

**Check:**
- `curl -I <base_url>` — verify the endpoint is reachable.
- For local models (Ollama, LM Studio, vLLM), confirm the server is running
  and listening on the expected port:
  ```bash
  curl http://localhost:11434/api/tags   # Ollama
  ```
- Check for network proxies — the core uses `reqwest` with system proxy settings.

**Fix:**
- Start your local model server if it's not running.
- Correct the `base_url` in the provider configuration.
- Check firewall rules — local model servers often bind to `127.0.0.1`.

---

## Provider Errors

### "Provider returned empty model list"

**Symptom:** The model picker shows the provider but no models are listed.

**Likely cause:** The provider's `/models` endpoint returned an empty or
unexpected response format.

**Check:**
- `curl <base_url>/models` to see the raw response.
- Some providers require authentication to list models — ensure the API key
  is set.

**Fix:**
- Verify the provider's discovery endpoint.
- For non-standard providers, a code branch in `core/src/provider.rs` may be
  needed (see the `add-key-provider` skill).

### Login flow never completes (OAuth providers)

**Symptom:** The browser opens for OAuth login but the TUI doesn't detect
completion, or the provider stays "not logged in."

**Likely cause:** Port forwarding, firewall, or browser security blocking the
loopback redirect.

**Check:**
- Ensure the OAuth callback port is not blocked by a firewall.
- Check browser developer tools for mixed-content or CORS warnings.
- Verify `localhost` resolves correctly (`/etc/hosts`).

**Fix:**
- Use the **manual code** option in the OAuth flow (copy-paste the code
  instead of relying on the callback).
- Check network proxies that might interfere with localhost routing.
- Ensure no other service is using the OAuth callback port.

---

## Tool Failures

### "Path escapes workspace" or "Path not allowed"

**Symptom:** A tool call (read_file, write_file, edit, glob, etc.) fails with
a path confinement error.

**Cause:** Catalyst Code enforces workspace confinement — all file paths must
resolve to within the workspace directory. Absolute paths, `..`, and symlink
escapes are rejected.

**Check:**
- Is the path relative? The tool requires workspace-relative paths.
- Does the path resolve to a location outside the workspace?
- Are there symlinks that point outside the workspace?

**Fix:**
- Use paths relative to the workspace root.
- Do not use absolute paths or `..` traversal.
- Access files outside the workspace via `bash` commands (with approval gate).

### "sandbox not available" or sandbox execution fails

**Symptom:** `bash` tool calls fail with a sandbox-related error.

**Likely cause:** The configured sandbox (`firejail`, `seatbelt`) is not
installed on the system.

**Check:**
- Check the sandbox configuration:
  ```
  /set sandbox none
  ```
- Verify the sandbox binary exists:
  ```bash
  which firejail   # Linux
  which sandbox-exec  # macOS
  ```

**Fix:**
- Set sandbox to `none` if you don't need it (the default):
  ```bash
  catcode --sandbox none
  ```
  or at runtime: `/set sandbox none`
- Install firejail: `apt install firejail` or `brew install firejail`
- On macOS, sandbox-exec is built-in but the default remains `none` unless
  configured.

### Tool hangs or times out

**Symptom:** A tool call (bash, fetch, search) does not complete.

**Likely cause:** The tool's operation is blocked (network, user input in bash,
infinite loop) or exceeds the timeout.

**Fix:**
- Use `/abort` to cancel the in-flight turn.
- Long-running operations should be split into smaller tool calls.
- Check network connectivity for `fetch` and search tools.

---

## Plugin Issues

### Plugin not loaded or not found

**Symptom:** A plugin in `.catalyst-code/plugins/` does not appear or its
hooks do not fire.

**Check:**
- Verify the plugin directory structure:
  ```
  .catalyst-code/plugins/<plugin-name>/
    plugin.json
  ```
- Check plugin.json for syntax errors: `cat .catalyst-code/plugins/<name>/plugin.json`
- Look for load errors in the core's debug log.

**Fix:**
- Ensure `plugin.json` follows the plugin schema exactly (see
  [Plugin Authoring Guide](plugins/index.md)).
- Restart the TUI so the core re-scans the plugins directory.
- Check the plugin's required dependencies (e.g. a runtime for hook scripts).

### Plugin hook failing silently

**Symptom:** A plugin hook should fire but nothing happens, or the turn
completes without the hook's side effects.

**Check:**
- The plugin system runs hooks fail-open — a failing hook does not block the
  turn. Check the debug log for hook errors.
- Verify the hook name is correct (`session_start`, `session_stop`,
  `pre_compact`, `pre_turn`, `pre_<tool>`, `post_<tool>`).

**Fix:**
- Add error logging to the hook script to capture failure details.
- Test the hook script independently: `bash .catalyst-code/plugins/<name>/hook-script.sh`

### Plugin permissions

**Symptom:** A plugin cannot access files or resources it needs.

**Cause:** Plugins run with the same permissions as the core process. If the
core is sandboxed, the plugin is also sandboxed.

**Fix:**
- Set appropriate file permissions for files the plugin needs to read/write.
- If the plugin needs network access, ensure the core's network policy allows
  it.
- Plugins that write to the workspace inherit workspace confinement rules.

---

## Session Issues

### Session won't load: "newer than supported"

**Symptom:** On startup or when loading a session, the core emits an error:
> session file <path> is version X, newer than supported (1); not loaded to
> avoid corrupting it.

**Cause:** The session file was created by a newer version of Catalyst Code
that uses an incompatible message format.

**Check:**
- Check the file's first line:
  ```bash
  head -1 <session-file>.jsonl
  ```
- Compare your binary version: `catcode-core --version`

**Fix:**
- Upgrade Catalyst Code to the version that created the session.
- If upgrade is not possible, delete the session file to start fresh (the old
  file can be archived for later migration).
- A migration tool is not yet available — the `_session_version` mechanism is
  designed to prevent silent data loss until one exists.

### Session appears empty or truncated after crash

**Symptom:** After a crash (power loss, kill -9), the session loads but the
last turn's messages are missing.

**Cause:** This is **by design** — the crash-safety model guarantees at most
one lost in-flight turn. Messages are flushed to the kernel during the turn
but are only `fsync`'d at turn end. A crash between turn start and turn-end
fsync loses the in-progress messages.

**Check:**
- The session file ends at the last successfully completed turn. Earlier
  messages are intact because the append + atomic rewrite pattern protects them.
- Only the (incomplete) turn that was in flight at crash time is missing.

**Fix:**
- Use `/undo` after restart if the filesystem was also affected (restores from
  the latest auto-checkpoint).
- No data recovery is possible for the lost turn — it was never committed to
  disk.

### Session file locked

**Symptom:** Cannot delete or overwrite a session file. The TUI shows "session
is active in another process."

**Cause:** A `.lock` sidecar file indicates another Catalyst Code instance is
using this session.

**Check:**
- Look for `.lock` files beside the session:
  ```bash
  ls -la <session-file>.lock
  ```
- Check the PID in the lock file: `cat <session-file>.lock`

**Fix:**
- Close the other instance, or kill the stale process if it crashed without
  cleaning up:
  ```bash
  kill <pid>
  ```
- If the process is already dead, remove the `.lock` file manually.
- The lock prevents two instances from corrupting the same session file;
  removing it when no other process is active is safe.

### Session deleted by mistake

**Check:**
- Session deletion removes the `.jsonl`, `.meta.json`, `.stats`, and
  `.meta.lock` files.
- No built-in trash/recycle bin exists.

**Fix:**
- If you have filesystem snapshots or backup, restore from there.
- Sessions are stored at `~/.config/catalyst-code/sessions/<hash>/`.

---

## Web Frontend Issues

### "port already in use"

See [Port conflict](#port-conflict-web-frontend) above.

### Browser shows "This site can't be reached"

**Likely cause:** The web service is not running or is bound to a different
host/port.

**Check:**
- Is the service running? Look for `catcode` or `catcode-core` in process list.
- What port was it configured with? Default: `49283`.
- Is it listening on `0.0.0.0` or `127.0.0.1`?

**Fix:**
- Start the service: `catcode` (TUI) automatically starts the web backend.
- If running headless: `catcode-core serve --port 49283`
- If the service is bound to `127.0.0.1`, access it only from the same machine.

### Mixed content: WebSocket blocked

**Symptom:** The web UI loads but the model response never arrives. Browser
dev console shows "Mixed Content: The page was loaded over HTTPS but attempted
to connect to an insecure WebSocket endpoint `ws://...`"

**Cause:** When the panel is served over HTTPS (e.g. via a reverse proxy like
Caddy/nginx), the WebSocket connection to the core must also be `wss://`
instead of `ws://`.

**Fix:**
- Serve the frontend over HTTP on the local machine (the default) — there's
  no sensitive data in transit over localhost.
- If you must use HTTPS, configure a WebSocket proxy that upgrades the
  connection to WSS.
- Or use the TUI instead of the web frontend — the TUI connects directly over
  stdio, avoiding the issue entirely.

### Node.js / Bun not found

**Symptom:** The web service fails to start because Node.js or Bun is missing.

**Cause:** The prebuilt web bundle needs a JavaScript runtime to serve files
(statically, with the API proxy). The runtime is not bundled with the
installer.

**Check:**
- `node --version` or `bun --version`
- The preset install path for the web server binary.

**Fix:**
- Install Node.js (>=18) or Bun. Either works.
- The installer checks for these at setup time and warns if neither is found.
- Fall back to the TUI-only install (`install.sh` without `--with-web`).

---

## Update Issues

### `catcode --update` does nothing

**Symptom:** Running `catcode --update` exits silently or says "up to date"
but you know a newer release exists.

**Check:**
- Run `catcode --version` to see the current version.
- Check the [releases page](https://github.com/catalystctl/catcode/releases)
  for the latest version.
- Run the installer with `--update` flag:
  ```bash
  curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install.sh | bash -s -- --update
  ```

**Fix:**
- The `--update` flag re-downloads and reinstalls the CLI, core, and web
  bundle (if installed), then restarts the service.
- If the service is managed by systemd/launchd/NSSM, the updater restarts it
  automatically.
- On Windows, use `install.ps1 -Update`.

### Update fails with checksum mismatch

**Symptom:** The installer downloads a release but fails checksum verification.

**Cause:** Corrupted download or man-in-the-middle.

**Fix:**
- Retry the update — the error is usually a transient network issue.
- Specify a specific version to pin downloads:
  ```bash
  ... | bash -s -- --version 0.2.0
  ```
- Check the release checksums against the published values on GitHub.

---

## Diagnostic Commands

| Command | What it shows |
|---------|---------------|
| `/stats` | Cumulative tokens, turns, cache ratio, context size |
| `/usage` | Provider plan and rate-limit windows |
| `/models` | Available models across all providers |
| `/sessions` | Session picker with metadata |
| `/memory` | Memory catalog |
| `/compact` | Manual context compaction |
| `/set` | Current runtime configuration |
| `catcode --version` | Binary version |

For deeper diagnostics, check the debug log at `~/.config/catalyst-code/logs/`
or run with `RUST_LOG=debug`.

---

## Still Stuck?

- Open a [GitHub issue](https://github.com/catalystctl/catcode/issues/new)
- Check existing issues for similar symptoms.
- Include the output of `/stats`, `/sessions`, and the core debug log.
- For crash bugs, include the core's stderr output and the session file header
  (first line of the active `.jsonl`).
