package main

import (
	"reflect"
	"testing"

	"charm.land/bubbles/v2/textinput"
)

// TestMultilineInputReflectionTargetExists guards the reflection in
// enableMultilineInput, which swaps textinput's unexported `rsan` sanitizer
// for a passthrough that preserves newlines (so Shift+Enter / pasted
// multi-line text survive). The sanitizer field has no public setter, so we
// write it via reflect.NewAt. If a bubbles upgrade renames or removes `rsan`,
// the function silently degrades to single-line input (no crash) — this test
// makes that regression loud at test time instead of a discovered UX bug.
//
// bubbles v2 moved runeutil to an internal package, so enableMultilineInput
// installs its own passthroughSanitizer rather than constructing runeutil's;
// if this test fails after an upgrade, update the reflection to the new field.
func TestMultilineInputReflectionTargetExists(t *testing.T) {
	m := textinput.New()
	v := reflect.ValueOf(&m).Elem()
	f := v.FieldByName("rsan")
	if !f.IsValid() {
		t.Fatal("textinput.Model no longer has an 'rsan' field — " +
			"enableMultilineInput is now a silent no-op; update the reflection")
	}
	if !f.CanAddr() {
		t.Fatal("textinput.Model.rsan is not addressable — " +
			"enableMultilineInput's write cannot proceed")
	}
}
