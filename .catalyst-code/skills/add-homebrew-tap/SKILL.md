---
name: add-homebrew-tap
description: Add Homebrew distribution (cask + formula) for a project that ships prebuilt CLI binaries. Creates placeholder templates, a render script, a release-automation workflow gated to v* tags, and a release-workflow version fix; web/service components go via caveats.
version: 1
---

## When to use

The user asks for "brew" / "homebrew" / "make a cask" for a CLI that is already
published to GitHub Releases as prebuilt binaries (standalone executable or
`.dmg`, **not** a `.app` bundle). Also when they say "ship both" or want
`brew install` plus `brew install --cask`.

Key framing to surface (the user often says "cask" loosely): a CLI binary with no
`.app` is idiomatic as a **formula** (`brew install <tool>`); a **cask**
(`brew install --cask <tool>`) also works in a personal tap via the `binary`
stanza. Default recommendation = ship **both** in a `homebrew-<tool>` tap (no
name conflict; unqualified `brew install` resolves to the formula). Confirm with
the user before building — it's a consequential, user-facing choice.

Web/service frontends (need a runtime + background service + config) **cannot**
be a cask/formula — wire them via `caveats` pointing to the one-line installer.

## Steps

1. **Confirm artifacts.** Grep the release script(s) + `install.sh` for the exact
   asset names and versioning. You need: per-arch macOS standalone binary name
   (e.g. `catcode-<ver>-macos-{arm64,x86_64}`) and the `.dmg` name, the GitHub
   repo slug, the `--version` flag output (for the formula `test` block), and
   where the CLI caches config (for `zap`).

2. **Check version/asset-name consistency.** If the release workflow names
   artifacts with the commit SHA even for `v*` tags (common bug: `version=git
   rev-parse --short HEAD`), `install.sh` AND brew break. Fix the "Resolve
   version" step so `v*` tags produce semver-named assets:
   `version="${GITHUB_REF_NAME#v}"` on `refs/tags/v*`, else SHA. Apply to every
   job that has that step.

3. **Create templates** at `packaging/homebrew/Casks/<tool>.rb` and
   `packaging/homebrew/Formula/<tool>.rb` with `@@VERSION@@`,
   `@@SHA256_ARM@@`, `@@SHA256_INTEL@@` placeholders (valid Ruby pre-render):
   - Cask: `arch arm:/intel:`, `url "...#{version}/...-#{arch}.dmg"`, per-arch
     `sha256`, `binary "<tool>"`, `caveats` (web one-liner), `zap trash: [...]`.
   - Formula: `on_macos do on_arm/on_intel do url "...-arm64"; sha256 end end`,
     `bin.install Dir["<tool>-*"].first => "<tool>"` (raw binary, not an
     archive), `livecheck { strategy :github_latest }`, `test do` using
     `--version`, `caveats`.

4. **Write `packaging/homebrew/render.sh`** that takes `<version> <arm-bin-sha>
   <intel-bin-sha> <arm-dmg-sha> <intel-dmg-sha> <out-dir>` and `sed`s the
   placeholders. **Cask gets the DMG hashes; formula gets the BINARY hashes.**
   Validate 64-hex sha256.

5. **Write `.github/workflows/homebrew-tap.yml`** on `release: published` +
   `workflow_dispatch`, **gated to `v*` tags** (SHA versions aren't monotonic →
   `brew upgrade` can't rank them). It: `gh release download` the per-arch
   assets, sha256sums them, runs render.sh, clones the tap repo, copies
   `Casks/`+`Formula/` (+ seeds README on first publish), commits, pushes via a
   `HOMEBREW_TAP_TOKEN` secret.

6. **Document.** Add a Homebrew install section to the README. Seed a tap README
   (`packaging/homebrew/tap-README.md`).

7. **Verify locally:** `bash -n render.sh`; YAML-parse both workflows; run
   render.sh with fake 64-hex hashes and check `do`/`end` balance, heredoc
   closure, and that placeholders are gone (except the descriptive comment).
   `ruby -c` if ruby is available (often it isn't on the build host — eyeball).

8. **Tell the user the one-time manual setup:** create an empty PUBLIC
   `homebrew-<tool>` repo + add a `HOMEBREW_TAP_TOKEN` PAT (contents-write)
   secret. Then a `v*` tag release auto-populates the tap.

## Example

catcode: cask (`brew install --cask catcode`, installs the `.dmg` via
`binary "catcode"`) + formula (`brew install catcode`, raw prebuilt binary).
Templates at `packaging/homebrew/{Casks,Formula}/catcode.rb`; render.sh;
`.github/workflows/homebrew-tap.yml`; release.yml `v*`-tag version fix. See
memories `homebrew-tap-cask-formula` (workspace) and
`homebrew-tap-pattern-prebuilt-cli` (global) for the full instance + pattern.
