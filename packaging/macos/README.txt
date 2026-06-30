ucli <VERSION> for macOS (<ARCH>)
================================

ucli is the terminal interface for the Umans AI coding-agent harness. This .dmg
ships a single self-contained `ucli` executable — the Rust core is embedded,
so there's no separate umans-core to install.

Install (easiest)
----------------
Double-click "Install ucli.command" and follow the prompt. It copies `ucli`
to /usr/local/bin (you may be asked for your password). Then open a NEW
terminal and run:

    ucli

Install (manual)
---------------
Drag `ucli` into a directory on your PATH, e.g.:

    sudo cp ucli /usr/local/bin/ucli
    sudo chmod +x /usr/local/bin/ucli

Or run it directly from anywhere without installing:

    ./ucli

First run inside ucli
--------------------
    /key sk-...        set your Umans API key (https://app.umans.ai/billing)
    /model             list models / pick one (e.g. /model glm-5.2)
    <type a prompt>    chat with the agent

Notes
-----
- The workspace is the directory you launch ucli from; rerun from another
  folder to work on a different project.
- Sandboxing (--sandbox firejail / --no-network) is Linux-only; leave /sandbox
  set to none.
- The agent's bash tool needs bash on PATH (present by default on macOS).
