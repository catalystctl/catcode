---
name: e2e-test-core-tool
description: Rigorously end-to-end test a new async core tool (one that shells out to real processes/infra) — #[ignore] tokio test against live deps + bash replication script + honest verification report
version: 1
---

## When to use

The user asks "has this been e2e tested?" / "do e2e tests" for a tool you added
(anything in `core/src/tools.rs` / a new `core/src/<tool>.rs` that spawns real
processes — containers, VMs, SSH, QEMU, etc.). Unit tests of pure logic are NOT
e2e. This skill is how you actually exercise the tool's real code path against
real infrastructure and report honestly.

## Workflow

1. **Probe the host first.** Before promising e2e, check the deps the tool
   shells out to actually exist (`command -v podman qemu-system-x86_64 swtpm
   websockify ...`, `ls -la /dev/kvm`). State what's feasible here vs. not.
   Don't claim e2e for a path whose runtime deps are missing.

2. **Build the real artifact** the tool depends on (e.g. `podman build -t
   catalyst/<img>:<tag> packaging/.../`). This validates the Dockerfile /
   install pipeline itself.

3. **Bash replication script** (`tmp/e2e-<name>.sh`) that runs the EXACT
   command sequence the Rust tool builds — same argv, same parsing of output.
   This validates the infrastructure + command shapes the tool relies on, with
   clear PASS/FAIL per step. It catches bugs the Rust glue would hit (e.g.
   `podman -p 0:5900` rejected, `localhost`→IPv6 misses rootless pasta).

4. **Real Rust dispatch test** — the key step. Add an `#[ignore]
   #[tokio::test]` in the tool's `#[cfg(test)] mod tests` that calls the actual
   `execute_<tool>(&json!({...}), &cfg)` against live infra:
   ```rust
   #[ignore]
   #[tokio::test]
   async fn e2e_<tool>_<scenario>() {
       // skip cleanly if the runtime dep is absent
       if !std::process::Command::new("podman").arg("image").arg("exists")
           .arg("catalyst/<img>").status().map(|s| s.success()).unwrap_or(false) {
           eprintln!("skipping: image not present"); return;
       }
       let cfg = Config::default(); // most tools ignore cfg for the live path
       let r = execute_tool(&json!({"action":"create",...}), &cfg).await;
       assert!(r.ok, "{}", r.output);
       // ... exec / screenshot / destroy, parsing r.output (JSON string) ...
       // DROP GUARD so a panicking assert still cleans up the real resource:
       struct Guard(Option<String>);
       impl Drop for Guard { fn drop(&mut self) {
           if let Some(id) = self.0.take() {
               let _ = std::process::Command::new("podman").args(["rm","-f",&id]).output();
           }
       }}
       let mut guard = Guard(Some(id.clone()));
       // ... assertions ...
       guard.0.take(); // disarm after explicit destroy
   }
   ```
   Run with: `cargo test --bin core <tool> -- --ignored --nocapture`
   (NOT plain `cargo test` — see gotchas).

5. **Fix bugs found**, re-verify (`cargo check` + `cargo clippy` + re-run the
   ignored test + `bun run typecheck` for any web changes).

6. **Honest report**: what passed (with the real output), bugs found+fixed, and
   explicitly what is STILL NOT e2e tested (paths whose deps are missing, or
   browser-side behavior you can't drive). Never imply "verified" beyond what
   you actually ran.

## Gotchas

- **`cargo test` (full) is broken by pre-existing oauth.rs/Cargo.toml
  working-tree changes** — use `cargo test --bin core <name>` (the binary's
  unit tests compile fine; only the full test target hits oauth.rs). See memory
  `oauth-test-binary-preexisting-broken`.
- **`#[ignore]` + `--ignored`** runs only ignored tests (filtered by name), so
  no parallel-env-var races with the rest of the suite. Don't mutate
  process-global env vars in the test (read defaults instead). See
  `env-var-mutation-races-parallel-tests`.
- **Concurrent sessions**: if other agent sessions are editing the tree, web
  `bun run typecheck` may show errors in files you never touched. Isolate:
  confirm YOUR files have zero errors (`typecheck | grep <your-file>`), and
  check `git show HEAD:<file>` to prove a breakage is a concurrent working-tree
  change, not yours. See `concurrent-user-edits-isolate-errors`.
- **`localhost` vs `127.0.0.1`**: rootless podman's `pasta`/`slirp4netns`
  forwarder listens on IPv4 only; `localhost`→`::1` fails. Use `127.0.0.1` in
  any URL a browser/curl connects to. See `podman-port-0-and-localhost-ipv6-gotchas`.
- **Outcome is a string**: `execute_<tool>` returns `Outcome { ok, output:
  String, diff }`. For create-style actions, `output` is a JSON string —
  `serde_json::from_str::<Value>(&r.output)` to get fields. To return an IMAGE
  the model can see, you need `Outcome.image` + multimodal tool-result support
  (Message::Tool content is String-only today) — a separate, delicate change.
