# Homebrew formula for Catalyst Code — installs the standalone macOS `catcode`
# binary (the TUI with the Rust core embedded) per arch.
#
# TEMPLATE: the `@@...@@` placeholders are filled in by render.sh / the
# homebrew-tap workflow on each vX.Y.Z release and the rendered file is pushed
# to the catalystctl/homebrew-catcode tap. Placeholders are valid Ruby so this
# file still parses if audited in-tree.
#
# Install (once the tap exists):
#   brew tap catalystctl/catcode
#   brew install catcode
class Catcode < Formula
  desc "Self-hosted, OpenAI-compatible coding-agent harness"
  homepage "https://github.com/catalystctl/catcode"
  license "MIT"
  version "@@VERSION@@"

  on_macos do
    on_arm do
      url "https://github.com/catalystctl/catcode/releases/download/v#{version}/catcode-#{version}-macos-arm64"
      sha256 "@@SHA256_ARM@@"
    end
    on_intel do
      url "https://github.com/catalystctl/catcode/releases/download/v#{version}/catcode-#{version}-macos-x86_64"
      sha256 "@@SHA256_INTEL@@"
    end
  end

  livecheck do
    url :homepage
    strategy :github_latest
  end

  # The download is a single prebuilt standalone binary (the Rust core is
  # embedded via go:embed), not an archive, so place it directly on PATH.
  def install
    bin.install Dir["catcode-*"].first => "catcode"
  end

  # `--version` is handled before the TUI/core launch, so it runs headless and
  # prints "catcode <version>".
  test do
    assert_match "catcode", shell_output("#{bin}/catcode --version")
    assert_match version.to_s, shell_output("#{bin}/catcode --version")
  end

  caveats <<~EOS
    Catalyst Code CLI is installed. Run `catcode` from any terminal, then
    `/login` and `/model` to start.

    Optional web frontend (needs Node or Bun; not part of this formula):
      curl -fsSL https://raw.githubusercontent.com/catalystctl/catcode/refs/heads/master/install-web.sh | bash
  EOS
end
