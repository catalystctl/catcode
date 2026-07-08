package main

import (
	"reflect"
	"testing"

	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"
)

// TestMultilineInputReflectionTargetExists guards the unsafe reflection in
// enableMultilineInput, which swaps textinput's unexported `rsan` sanitizer
// for one that preserves newlines (so Shift+Enter / pasted multi-line text
// survive). The sanitizer field has no public setter, so we write it via
// unsafe.Pointer. If a bubbles upgrade renames or removes `rsan`, the
// function silently degrades to single-line input (no crash) — this test
// makes that regression loud at test time instead of a discovered UX bug.
//
// bubbles is pinned at v1.0.0; if this test fails after an upgrade, either
// re-pin or update enableMultilineInput to the new field name.
func TestMultilineInputReflectionTargetExists(t *testing.T) {
	m := textinput.New()
	v := reflect.ValueOf(&m).Elem()
	f := v.FieldByName("rsan")
	if !f.IsValid() {
		t.Fatal("textinput.Model no longer has an 'rsan' field — " +
			"enableMultilineInput is now a silent no-op; update the reflection or re-pin bubbles")
	}
	if !f.CanAddr() {
		t.Fatal("textinput.Model.rsan is not addressable — " +
			"enableMultilineInput's unsafe write cannot proceed")
	}
}

// TestModifiedEnterCSIClassification ensures the reflection in isModifiedEnterCSI
// (which reaches into bubbletea's unexported unknownCSISequenceMsg []byte type)
// classifies correctly and never panics for the ordinary message kinds the TUI
// receives. A bubbles/bubbletea upgrade that changes how modified-Enter arrives
// must not crash the loop or misclassify regular keys.
func TestModifiedEnterCSIClassification(t *testing.T) {
	// Non-CSI / ordinary messages: must return false and never panic.
	nonCSI := []tea.Msg{
		tea.KeyMsg{Type: tea.KeyEnter},
		tea.KeyMsg{Type: tea.KeyCtrlC},
		tea.MouseMsg{},
		nil,
		"not a csi",
		[]byte("hello"),
		[]byte(""),
		[]byte("\x1b[5n"), // a different CSI (device status), not modified-Enter
	}
	for _, c := range nonCSI {
		if isCtrlEnterUnknownCSI(c) {
			t.Errorf("ctrl-enter: expected false for %T, got true", c)
		}
		if isShiftEnterUnknownCSI(c) {
			t.Errorf("shift-enter: expected false for %T, got true", c)
		}
	}
	// Modified-Enter CSI byte sequences: must classify true (the function matches
	// any []byte whose content is the CSI — it can't see the unexported type).
	if !isShiftEnterUnknownCSI([]byte("\x1b[13;2u")) {
		t.Error("shift-enter Kitty CSI not recognized")
	}
	if !isCtrlEnterUnknownCSI([]byte("\x1b[27;5;13~")) {
		t.Error("ctrl-enter xterm CSI not recognized")
	}
}
