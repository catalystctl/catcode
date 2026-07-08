---
name: setup-self-hosted-gh-runner
description: Set up a SECURE self-hosted GitHub Actions runner using ephemeral rootless Podman containers (fresh container per job, destroyed after) driven by a lingered systemd user service. Use when GitHub-hosted runners are too slow / too costly and the box runs Linux with Podman.
---

# Set up a secure self-hosted GitHub Actions runner (ephemeral rootless Podman)

## When to use

- GitHub-hosted runners are slow (cold caches + queue/provisioning) and you have a Linux box with Podman.
- You want ISOLATION: each CI job runs in a fresh container destroyed after the job — no persistence for backdoors, no root daemon, no access to the host's home/secrets/other services.
- Repo can be public or private; rootless Podman + ephemeral containers are secure enough for either (the high-value credential never enters a container).

Prefer this over bare-metal runners (no isolation) or Docker-based (needs a root daemon). Skip if the box has no Podman (install it + `loginctl enable-linger`), or if jobs need kernel features unavailable in a rootless userns (FUSE mounts, AppImage, certain device access) — keep THOSE jobs on `ubuntu-latest`.

## Architecture

```
systemd user service (gh-runner.service, lingered)
  └─ supervisor.sh            keeps N ephemeral containers alive
       ├─ slot 0: podman run --rm --ephemeral  -v gh-cache-0:/cache  gh-runner
       └─ slot 1: podman run --rm --ephemeral  -v gh-cache-1:/cache  gh-runner
```

- **Ephemeral**: `config.sh --ephemeral` runs exactly ONE job then auto-deregisters; `--rm` destroys the container; the supervisor respawns a fresh one.
- **Rootless**: container `root` (uid 0) is mapped to the unprivileged host user via the user namespace — jobs have no real host privileges.
- **Token hygiene**: supervisor mints a fresh ~1h registration token per spawn via `gh api -X POST`; the high-value `gh` PAT stays on the host and NEVER enters a container.
- **Per-slot cache volumes** (not shared) so concurrent containers don't race on cargo/go caches. Mount ONLY pure caches (GOMODCACHE, GOCACHE, RUNNER_TOOL_CACHE, pip/npm); keep toolchain HOMES (RUSTUP_HOME, CARGO_HOME, /usr/local/go) in the IMAGE so the volume overlay never hides installed tools.

## Steps

1. **Recon.** Confirm Podman rootless (`podman info --format '{{.Host.Security.Rootless}}'`), subuid/subgid (`grep $USER /etc/subuid /etc/subgid`), linger (`loginctl show-user $USER | grep Linger` → enable-linger if off), `gh auth status` (needs `repo`+`workflow` scopes), and the repo's CI workflows (`runs-on: ubuntu-latest` jobs to convert). Note any job needing kernel features rootless can't provide (FUSE/AppImage, device access) → leave on ubuntu-latest.

2. **Clean up prior attempts.** If `~/actions-runner` exists from a stale/misconfigured setup, deregister its offline runner via `gh api -X DELETE repos/<o>/<r>/actions/runners/<id>` and remove stale `.runner`/`.credentials`.

3. **Build the runner image** (Containerfile, ubuntu:24.04 base for action compatibility):
   - System packages: `build-essential pkg-config libssl-dev ca-certificates git curl jq zip file sudo python3-pip` + the toolchains the repo's CI needs (e.g. Rust via rustup, Go, Node/Bun) + `buildah` if a CI job builds the repo's own Dockerfile.
   - Toolchain HOMES in the image (e.g. `ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo`); point only PURE caches at `/cache` (`ENV GOMODCACHE=/cache/go-mod GOCACHE=/cache/go-build RUNNER_TOOL_CACHE=/cache/toolcache ...`).
   - `ADD` the locally-downloaded actions-runner tarball to `/runner` (reuse a downloaded tarball to avoid version guessing).
   - See GOTCHAS below — three are MANDATORY in the image or the runner won't start.

4. **entrypoint.sh**: `config.sh --unattended --url https://github.com/<o>/<r> --token $REGISTRATION_TOKEN --name $RUNNER_NAME --labels $LABELS --ephemeral --work _work` then `./run.sh`; `trap './config.sh remove --token $REGISTRATION_TOKEN' EXIT`; `mkdir -p` the /cache subdirs at start (Podman named volumes don't always copy image content on first mount).

5. **supervisor.sh**: bash loop keeping CONCURRENCY slots alive. Per spawn: `gh api -X POST repos/<o>/<r>/actions/runners/registration-token --jq .token` (POST!), then `podman run --rm --name <prefix>-<host>-<slot> --memory 7g --memory-swap 0 --cpus 6 --pids-limit 1024 -e REPO/REGISTRATION_TOKEN/RUNNER_NAME/LABELS/RUNNER_ALLOW_RUNASROOT -v gh-cache-<slot>:/cache <image>`. On exit, respawn after 5s. A `cleanup_offline()` deletes leaked offline runners for the repo (safe — only your runners are on a private repo). Each slot in its own background `&` loop; `wait` blocks.

6. **systemd user unit** (`~/.config/systemd/user/gh-runner.service`): `Type=simple`, `Restart=always`, `Environment=` for REPO/CONCURRENCY/IMAGE/LABELS/PATH/HOME, `ExecStart=%h/gh-self-hosted/supervisor.sh`, `WantedBy=default.target`. `systemctl --user daemon-reload && systemctl --user enable --now gh-runner`.

7. **Convert the repo's CI workflows**: `runs-on: ubuntu-latest` → `runs-on: [self-hosted, Linux, X64]`. If a job uses `docker/buildx-action` (needs Docker), convert to `buildah build --isolation chroot --storage-driver vfs -t <tag> .` (works rootless-in-rootless). Commit on a branch + open a PR (pull_request triggers CI without touching master/release workflows).

8. **Verify end-to-end**: confirm runners `online` + `busy:false` via `gh api repos/<o>/<r>/actions/runners`; watch a real job land (`journalctl --user -u gh-runner` shows "Running job:" / "completed with result: Succeeded"). Expect the FIRST run to be cold (caches empty); subsequent runs fast.

## GOTCHAS (all MANDATORY — the runner will NOT start without these)

1. **Runner refuses root: "Must not run with sudo".** Rootless Podman maps host user → container root (uid 0); the runner bails on uid 0. FIX: env `RUNNER_ALLOW_RUNASROOT=1` (pass via `podman run -e`; bake at the END of the Containerfile so it doesn't invalidate cached layers).

2. **.NET listener crashes (exit 134): "Couldn't find a valid ICU package".** The runner listener is a .NET 6 app. FIX: `RUN cd /runner && ./bin/installdependencies.sh` in the image (installs libicu + liblttng-ust + krb5). The "Execute sudo ./bin/installdependencies.sh" message is FATAL, not a warning.

3. **buildah can't resolve short-name images: "short-name 'rust:1.82-slim' did not resolve... no unqualified-search registries".** FIX: write `/etc/containers/registries.conf` with `unqualified-search-registries = ["docker.io"]`.

## Other gotchas

4. **`systemctl --user` from a non-login shell (agent bash, cron) fails**: "$DBUS_SESSION_BUS_ADDRESS and $XDG_RUNTIME_DIR not defined". FIX: `export XDG_RUNTIME_DIR=/run/user/$(id -u) DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$(id -u)/bus` first. Requires `loginctl enable-linger <user>`.

5. **Registration token endpoint is POST.** `gh api repos/<o>/<r>/actions/runners/registration-token` defaults to GET. Use `gh api -X POST ... --jq .token`. Tokens live ~1h; mint fresh per spawn.

6. **Orphaned-job wedge**: killing a runner container mid-job leaves the job "in_progress" on an offline runner → blocks runner delete (422) AND blocks `gh run rerun` ("already running"). Recover: `gh run cancel <run>`, then trigger fresh (empty commit → new pull_request run); GitHub eventually clears it. Ephemeral + `--rm` + a `config.sh remove` trap prevents it in normal operation.

7. **Per-slot cache volumes, not shared.** Concurrent containers on the SAME volume race on cargo-registry/go-mod writes. Give each slot its own volume.

8. **Runner auto-update is fine in ephemeral mode** (downloads newer version, restarts within the container, completes cleanly). A brief "Not configured" crash-loop right after a supervisor restart is a TRANSIENT race (old containers tearing down + a concurrent rerun); it self-resolves — watch for 0 errors over ~20s rather than reacting to the first burst.

## Manage

```bash
systemctl --user status|restart|stop gh-runner
journalctl --user -u gh-runner -f                      # supervisor + runner logs
gh api repos/<o>/<r>/actions/runners --jq '.runners[]|{name,status,busy}'
# rebuild image:  cd ~/gh-self-hosted && podman build -t gh-runner . && systemctl --user restart gh-runner
# tune:           systemctl --user edit gh-runner  (Environment=CONCURRENCY=3 ...)
```
