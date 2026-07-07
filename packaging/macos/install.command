#!/bin/sh
# catcode macOS installer — copies the bundled standalone `catcode` (the TUI with the
# Rust core embedded, so it's one self-contained file) onto your PATH so you
# can run `catcode` from any terminal, in any directory.
#
# Double-click this file from inside the mounted catcode .dmg (it opens Terminal
# and runs). Or run it from a shell:  sh "Install catcode.command"
set -e
HERE="$(cd "$(dirname "$0")" && pwd)"
SRC="$HERE/catcode"
DEST_DIR="${UCLI_INSTALL_DIR:-/usr/local/bin}"
DEST="$DEST_DIR/catcode"

if [ ! -f "$SRC" ]; then
	echo "catcode binary not found next to this installer ($SRC)." >&2
	echo "Run this from inside the mounted catcode .dmg." >&2
	exit 1
fi

echo "Installing catcode to $DEST ..."
if [ -w "$DEST_DIR" ]; then
	cp "$SRC" "$DEST"
	chmod +x "$DEST"
else
	echo "(admin needed; enter your password if prompted)"
	sudo cp "$SRC" "$DEST"
	sudo chmod +x "$DEST"
fi

echo
echo "Done. Open a NEW terminal window (so PATH reloads) and run:"
echo "    catcode"
echo
echo "First run inside catcode:  /key sk-...   then /model, then type a prompt."
echo "The workspace is the directory you launch catcode from."
echo "Tip: the agent's bash tool needs bash on PATH (present by default on macOS)."
