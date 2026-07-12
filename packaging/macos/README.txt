catcode <VERSION> for macOS (<ARCH>)
================================

catcode is the terminal interface for the Catalyst Code harness. This .dmg
ships a single self-contained `catcode` executable — the Rust core is embedded,
so there's no separate catcode-core to install.

Install (easiest)
----------------
Double-click "Install catcode.command" and follow the prompt. It copies `catcode`
to /usr/local/bin (you may be asked for your password). Then open a NEW
terminal and run:

    catcode

Install (manual)
---------------
Drag `catcode` into a directory on your PATH, e.g.:

    sudo cp catcode /usr/local/bin/catcode
    sudo chmod +x /usr/local/bin/catcode

Or run it directly from anywhere without installing:

    ./catcode

First run inside catcode
--------------------
    /login             log in (API key or OAuth) — https://app.umans.ai/billing
    /model             list models / pick one (e.g. /model glm-5.2)
    <type a prompt>    chat with the agent

Notes
-----
- The workspace is the directory you launch catcode from; rerun from another
  folder to work on a different project.
- Sandboxing (--sandbox firejail / --no-network) is Linux-only; leave /sandbox
  set to none.
- The agent's bash tool needs bash on PATH (present by default on macOS).
