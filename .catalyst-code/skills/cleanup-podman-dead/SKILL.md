---
name: cleanup-podman-dead
description: Remove all exited podman containers and pods (rootless or rootful), handle dependency errors gracefully
---

# Clean up podman dead containers

## When to use

The user has accumulated dead/exited podman containers and wants them cleaned up. Recurring pattern (detected across sessions).

## Steps

1. **Audit** — list all exited containers and pods:
   ```
   podman ps -a --filter "status=exited" --format "table {{.Names}}\t{{.Status}}\t{{.CreatedAt}}"
   podman pod ps --filter "status=exited" --filter "status=created"
   ```

2. **Remove exited containers** (force, no interactive prompts):
   ```
   podman rm -f <name1> <name2> ...
   ```
   - Pass names from the audit output.
   - Do NOT use `--all` with explicit names — that's an error.

3. **Remove exited pods**:
   ```
   podman pod rm -f <pod1> <pod2> ...
   ```

4. **Handle dependency errors**: Some containers may fail with:
   ```
   Error: container X has dependent containers which must be removed before it
   ```
   Retry with the name alone (without --all). If still stuck, find and remove orphans:
   ```
   podman ps -a --filter "status=exited" --no-trunc 2>/dev/null
   ```

5. **Verify** — only running containers remain:
   ```
   podman ps --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"
   ```

## Notes

- Rootless containers live in the user session (`/run/user/$(id -u)`) — they only auto-start after login, never before boot.
- Rootful containers (`sudo podman`) can use `--restart=always` and start before login via system systemd units.
- If the user asks for "no login required on boot", rootful is the only option — rootless containers fundamentally need the user session.
- Storage reclaimed: check with `podman system df` — rootless setups often reclaim gigabytes from dead container layers.
