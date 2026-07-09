package main

import (
	"testing"

	tea "charm.land/bubbletea/v2"
)

// TestCtrlEnterMatchesSteerKeybind: in v2, modified Enter arrives as a real
// KeyPressMsg with modifier bits (Code: KeyEnter, Mod: ModCtrl). handleKey
// dispatches steer via kb(msg, "steer"), which compares msg.String() to the
// canonical "ctrl+enter" — so a real Ctrl+Enter must match and a plain Enter
// must not.
func TestCtrlEnterMatchesSteerKeybind(t *testing.T) {
	s := newInputSession(t, 60)

	// Ctrl+Enter -> String() == "ctrl+enter" -> matches the default steer keybind.
	if !s.kb(tea.KeyPressMsg{Code: tea.KeyEnter, Mod: tea.ModCtrl}, "steer") {
		t.Error("Ctrl+Enter should match the steer keybind (ctrl+enter)")
	}
	// A plain Enter must NOT match steer (it's the send key).
	if s.kb(tea.KeyPressMsg{Code: tea.KeyEnter}, "steer") {
		t.Error("plain Enter should not match the steer keybind")
	}
	// Shift+Enter must NOT match steer (it's the newline key).
	if s.kb(tea.KeyPressMsg{Code: tea.KeyEnter, Mod: tea.ModShift}, "steer") {
		t.Error("Shift+Enter should not match the steer keybind")
	}
}
