//go:build !embed_core

package main

// embeddedCorePath returns "" when no core is compiled into the binary (the
// normal dev / Linux / Windows builds). coreBinaryPath then falls back to its
// usual search of $UMANS_CORE and the dev/installed layouts.
func embeddedCorePath() string { return "" }
