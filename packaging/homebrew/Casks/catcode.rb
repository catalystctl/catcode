# Homebrew cask for Catalyst Code — installs the standalone macOS `catcode`
# binary (the TUI with the Rust core embedded) from the per-arch .dmg.
#
# TEMPLATE: the `@@...@@` placeholders are filled in by render.sh / the
# homebrew-tap workflow on each vX.Y.Z release and the rendered file is pushed
# to the catalystctl/homebrew-catcode tap. Placeholders are valid Ruby so this
# file still parses if audited in-tree.
#
# Install (once the tap exists):
#   brew tap catalystctl/catcode
#   brew install --cask catcode
cask "catcode" do
  arch arm: "arm64", intel: "x86_64"

  version "@@VERSION@@"
  sha256 arm:   "@@SHA256_ARM@@",
         intel: "@@SHA256_INTEL@@"

  url "https://github.com/catalystctl/catcode/releases/download/v#{version}/catcode-#{version}-macos-#{arch}.dmg"
  name "Catalyst Code"
  desc "Self-hosted, OpenAI-compatible coding-agent harness"
  homepage "https://github.com/catalystctl/catcode"

  livecheck do
    url :url
    strategy :github_latest
  end

  # The .dmg root ships a standalone `catcode` binary (the Rust core is
  # embedded via go:embed, so it is one self-contained file). Symlink it onto
  # PATH. The "Install catcode.command" + README in the .dmg are ignored.
  binary "catcode"

  caveats <<~EOS
    Catalyst Code CLI is installed. Run `catcode` from any terminal, then
    `/login` and `/model` to start.

    Optional web frontend (needs Node or Bun; not part of this cask):
      curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install-web.sh | bash
  EOS

  # `brew uninstall --zap catcode` also removes the embedded-core extraction
  # cache and the shared config/models cache. Per-workspace `.catalyst-code/`
  # directories are left in place — they belong to your projects.
  zap trash: [
    "~/Library/Caches/catalyst-code",
    "~/.config/catalyst-code",
  ]
end
