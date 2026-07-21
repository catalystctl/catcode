# catalystctl/homebrew-catcode

Homebrew tap for [Catalyst Code](https://github.com/catalystctl/catcode) — a
self-hosted, OpenAI-compatible coding-agent harness.

This tap is kept in sync automatically: every `vX.Y.Z` release of
`catalystctl/catcode` regenerates `Casks/catcode.rb` and `Formula/catcode.rb`
with the real version + per-arch sha256 (see
`.github/workflows/homebrew-tap.yml` in the main repo).

## Install

```bash
brew tap catalystctl/catcode
brew install catcode            # formula — `brew upgrade` keeps it current
# …or, if you prefer a cask:
brew install --cask catcode
```

Then run `catcode` from any terminal, `/login`, `/model`, and start prompting.

Both the formula and the cask install the **same standalone `catcode` binary**
(the Rust core is embedded, so it is one self-contained file). The formula is
the idiomatic Homebrew path for a CLI; the cask installs the same binary from
the `.dmg`. Pick one — don't install both.

## The optional web frontend

The web app needs a Node/Bun runtime + a background service, so it is **not**
part of either package. Install it separately with the one-liner:

```bash
curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install-web.sh | bash
```

Then open `http://localhost:49283`.

## Updating

`brew upgrade catcode` (or `brew upgrade --cask catcode`) pulls the latest
release. `catcode --update` (the built-in self-updater) also works, but using
brew to upgrade is recommended when installed via brew.

## Uninstall

```bash
brew uninstall catcode          # or: brew uninstall --cask catcode
brew untap catalystctl/catcode
```

`brew uninstall --zap catcode` additionally removes the embedded-core cache
(`~/Library/Caches/catalyst-code`) and the shared config/models cache
(`~/.config/catalyst-code`).
