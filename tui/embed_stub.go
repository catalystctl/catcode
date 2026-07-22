//go:build !embed_core

package main

// embeddedCoreAvailable reports whether a core is compiled into this binary
// (macOS standalone builds). When false the TUI shells out to a sibling
// catcode-core, which --update must then keep in sync with the CLI.
const embeddedCoreAvailable = false

// embeddedCorePath returns "" when no core is compiled into the binary (the
// normal dev / Linux / Windows builds). coreBinaryPath then falls back to its
// usual search of $CATCODE_CORE and the dev/installed layouts.
func embeddedCorePath() string { return "" }
