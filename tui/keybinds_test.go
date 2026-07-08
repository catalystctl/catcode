package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
)

// ---------------------------------------------------------------------------
// Keybind registry & merge
// ---------------------------------------------------------------------------

// TestDefaultKeybindsCoverAll ensures every keybindDef has a non-empty default
// and the map has exactly one entry per def.
func TestDefaultKeybindsCoverAll(t *testing.T) {
	m := defaultKeybinds()
	if len(m) != len(keybindDefs) {
		t.Fatalf("defaultKeybinds has %d entries, expected %d (one per def)", len(m), len(keybindDefs))
	}
	for _, d := range keybindDefs {
		v, ok := m[d.Action]
		if !ok {
			t.Errorf("action %q missing from defaultKeybinds", d.Action)
		}
		if v == "" {
			t.Errorf("action %q has empty default key", d.Action)
		}
		if v != d.Default {
			t.Errorf("action %q: map has %q, def has %q", d.Action, v, d.Default)
		}
	}
}

// TestEffectiveKeybindsMerge verifies user overrides layer over defaults and
// unknown actions are dropped.
func TestEffectiveKeybindsMerge(t *testing.T) {
	user := map[string]string{
		"quit":               "ctrl+q", // override
		"toggle_reasoning":   "ctrl+e", // override
		"nonexistent_action": "ctrl+z", // unknown — must be dropped
		"send":               "",       // empty — means DISABLED (not default)
	}
	m := effectiveKeybinds(user)
	if m["quit"] != "ctrl+q" {
		t.Fatalf("quit override: got %q, want ctrl+q", m["quit"])
	}
	if m["toggle_reasoning"] != "ctrl+e" {
		t.Fatalf("toggle_reasoning override: got %q, want ctrl+e", m["toggle_reasoning"])
	}
	// Unknown action should not appear.
	if _, ok := m["nonexistent_action"]; ok {
		t.Error("unknown action should be dropped from effective map")
	}
	// Empty override = disabled (kb returns false for it), NOT fall back to default.
	if m["send"] != "" {
		t.Fatalf("empty override should mean disabled: got %q, want empty string", m["send"])
	}
	// Untouched actions keep their defaults.
	if m["close"] != "esc" {
		t.Fatalf("untouched action close: got %q, want esc", m["close"])
	}
}

// TestEffectiveKeybindsNil ensures a nil user map yields pure defaults.
func TestEffectiveKeybindsNil(t *testing.T) {
	m := effectiveKeybinds(nil)
	if m["quit"] != "ctrl+c" {
		t.Fatalf("nil user map: quit=%q, want ctrl+c", m["quit"])
	}
	if len(m) != len(keybindDefs) {
		t.Fatalf("nil user map: %d entries, want %d", len(m), len(keybindDefs))
	}
}

// TestKeybindDefsUnique ensures no two defs share the same Action name (a
// duplicate would silently overwrite in the map).
func TestKeybindDefsUnique(t *testing.T) {
	seen := map[string]bool{}
	for _, d := range keybindDefs {
		if seen[d.Action] {
			t.Fatalf("duplicate action name: %q", d.Action)
		}
		seen[d.Action] = true
	}
}

// ---------------------------------------------------------------------------
// kb / kbAny matching
// ---------------------------------------------------------------------------

// TestKbMatch verifies the kb helper matches the bound key and is
// case-insensitive for single-character keys.
func TestKbMatch(t *testing.T) {
	s := initialSession()
	// Multi-char key: exact match.
	if !s.kb(keyMsg("ctrl+t"), "toggle_reasoning") {
		t.Error("kb should match ctrl+t for toggle_reasoning")
	}
	if s.kb(keyMsg("ctrl+e"), "toggle_reasoning") {
		t.Error("kb should NOT match ctrl+e for toggle_reasoning")
	}
	// Single-char key: case-insensitive.
	if !s.kb(keyMsg("y"), "approve") {
		t.Error("kb should match 'y' for approve")
	}
	if !s.kb(keyMsg("Y"), "approve") {
		t.Error("kb should match 'Y' for approve (case-insensitive)")
	}
	// Enter via tea.KeyMsg{Type: KeyEnter} has String()=="enter".
	if !s.kb(tea.KeyMsg{Type: tea.KeyEnter}, "send") {
		t.Error("kb should match KeyEnter for send")
	}
}

// TestKbAnyMatch verifies kbAny matches any of the listed actions.
func TestKbAnyMatch(t *testing.T) {
	s := initialSession()
	if !s.kbAny(keyMsg("ctrl+p"), "command_palette", "command_palette_alt") {
		t.Error("kbAny should match ctrl+p via command_palette")
	}
	if !s.kbAny(keyMsg("ctrl+k"), "command_palette", "command_palette_alt") {
		t.Error("kbAny should match ctrl+k via command_palette_alt")
	}
	if s.kbAny(keyMsg("ctrl+z"), "command_palette", "command_palette_alt") {
		t.Error("kbAny should NOT match ctrl+z")
	}
}

// TestKbWithRemappedKey verifies the kb helper respects a remapped binding.
func TestKbWithRemappedKey(t *testing.T) {
	s := initialSession()
	// Remap quit from ctrl+c to ctrl+q.
	s.keybinds["quit"] = "ctrl+q"
	if s.kb(keyMsg("ctrl+c"), "quit") {
		t.Error("kb should NOT match old ctrl+c after remap to ctrl+q")
	}
	if !s.kb(keyMsg("ctrl+q"), "quit") {
		t.Error("kb should match new ctrl+q after remap")
	}
}

// ---------------------------------------------------------------------------
// isBindableKey
// ---------------------------------------------------------------------------

func TestIsBindableKey(t *testing.T) {
	valid := []string{"ctrl+t", "enter", "esc", "y", " ", "ctrl+enter", "pgup", "shift+tab"}
	for _, k := range valid {
		if !isBindableKey(k) {
			t.Errorf("isBindableKey(%q) = false, want true", k)
		}
	}
	invalid := []string{"", "unknown", "error"}
	for _, k := range invalid {
		if isBindableKey(k) {
			t.Errorf("isBindableKey(%q) = true, want false", k)
		}
	}
}

// ---------------------------------------------------------------------------
// keybindLabel
// ---------------------------------------------------------------------------

func TestKeybindLabel(t *testing.T) {
	cases := map[string]string{
		"ctrl+t":     "Ctrl+T",
		"enter":      "Enter",
		"esc":        "Esc",
		" ":          "Space",
		"ctrl+enter": "Ctrl+Enter",
		"pgup":       "Pgup",
		"shift+tab":  "Shift+Tab",
		"y":          "Y",
	}
	for in, want := range cases {
		if got := keybindLabel(in); got != want {
			t.Errorf("keybindLabel(%q) = %q, want %q", in, got, want)
		}
	}
}

// ---------------------------------------------------------------------------
// applyKeybind / resetKeybind
// ---------------------------------------------------------------------------

// TestApplyAndResetKeybind verifies that rebinding updates the live keymap and
// the persisted settings, and that reset restores the default.
func TestApplyAndResetKeybind(t *testing.T) {
	s := initialSession()
	s.width, s.height = 100, 40
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")

	// Find the "quit" action index.
	quitIdx := -1
	for i, d := range keybindDefs {
		if d.Action == "quit" {
			quitIdx = i
			break
		}
	}
	if quitIdx < 0 {
		t.Fatal("quit action not found in keybindDefs")
	}

	// Apply a new binding.
	s.applyKeybind(quitIdx, "ctrl+q")
	if s.keybinds["quit"] != "ctrl+q" {
		t.Fatalf("after apply: quit=%q, want ctrl+q", s.keybinds["quit"])
	}
	if s.settings.Keybinds["quit"] != "ctrl+q" {
		t.Fatalf("after apply: settings.Keybinds[quit]=%q, want ctrl+q", s.settings.Keybinds["quit"])
	}

	// Reset to default.
	s.resetKeybind(quitIdx)
	if s.keybinds["quit"] != "ctrl+c" {
		t.Fatalf("after reset: quit=%q, want ctrl+c", s.keybinds["quit"])
	}
	if _, ok := s.settings.Keybinds["quit"]; ok {
		t.Error("after reset: settings.Keybinds[quit] should be deleted")
	}
}

// TestApplyKeybindOutOfBounds verifies out-of-range indices are safe no-ops.
func TestApplyKeybindOutOfBounds(t *testing.T) {
	s := initialSession()
	orig := s.keybinds["quit"]
	s.applyKeybind(-1, "ctrl+q")
	s.applyKeybind(len(keybindDefs), "ctrl+q")
	if s.keybinds["quit"] != orig {
		t.Fatal("out-of-bounds applyKeybind should be a no-op")
	}
}

// ---------------------------------------------------------------------------
// /keybinds modal
// ---------------------------------------------------------------------------

// TestKeybindsCommandOpensModal verifies /keybinds opens the modal.
func TestKeybindsCommandOpensModal(t *testing.T) {
	s := initialSession()
	s.width, s.height = 100, 40
	s.modal = modal{}
	_ = s.handleUserLine("/keybinds")
	if s.modal.kind != modalKeybinds {
		t.Fatalf("/keybinds should open modalKeybinds; got %v", s.modal.kind)
	}
}

// TestKeybindsModalNavigate verifies up/down navigation and enter starts capture.
func TestKeybindsModalNavigate(t *testing.T) {
	s := initialSession()
	s.width, s.height = 100, 40
	s.openKeybindsModal()
	n := len(keybindDefs)
	if s.modal.cursor != 0 {
		t.Fatalf("initial cursor should be 0; got %d", s.modal.cursor)
	}
	// Down once.
	_, _ = s.handleKeybindsKey(keyMsg("down"))
	if s.modal.cursor != 1 {
		t.Fatalf("after down: cursor=%d, want 1", s.modal.cursor)
	}
	// Up once.
	_, _ = s.handleKeybindsKey(keyMsg("up"))
	if s.modal.cursor != 0 {
		t.Fatalf("after up: cursor=%d, want 0", s.modal.cursor)
	}
	// Wrap around: up from 0 goes to last.
	_, _ = s.handleKeybindsKey(keyMsg("up"))
	if s.modal.cursor != n-1 {
		t.Fatalf("after wrap-up: cursor=%d, want %d", s.modal.cursor, n-1)
	}
	// Enter starts capture mode.
	_, _ = s.handleKeybindsKey(keyMsg("enter"))
	if !s.modal.editing {
		t.Fatal("enter should start capture mode (editing=true)")
	}
	// Esc cancels capture.
	_, _ = s.handleKeybindsKey(keyMsg("esc"))
	if s.modal.editing {
		t.Fatal("esc should cancel capture mode (editing=false)")
	}
}

// TestKeybindsModalCaptureRebind verifies the full rebind flow: navigate to an
// action, enter capture, press a key, confirm the binding changed.
func TestKeybindsModalCaptureRebind(t *testing.T) {
	s := initialSession()
	s.width, s.height = 100, 40
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.openKeybindsModal()

	// Navigate to quit (index 0, already there).
	// Enter capture.
	_, _ = s.handleKeybindsKey(keyMsg("enter"))
	if !s.modal.editing {
		t.Fatal("enter should start capture")
	}
	// Press ctrl+q to rebind.
	_, _ = s.handleKeybindsKey(keyMsg("ctrl+q"))
	if s.modal.editing {
		t.Fatal("after capture key, editing should be false")
	}
	if s.keybinds["quit"] != "ctrl+q" {
		t.Fatalf("after capture: quit=%q, want ctrl+q", s.keybinds["quit"])
	}
	// The kb helper should now match the new key.
	if !s.kb(keyMsg("ctrl+q"), "quit") {
		t.Error("kb should match remapped ctrl+q for quit")
	}
	if s.kb(keyMsg("ctrl+c"), "quit") {
		t.Error("kb should NOT match old ctrl+c for quit after remap")
	}
}

// TestKeybindsModalReset verifies backspace resets to default.
func TestKeybindsModalReset(t *testing.T) {
	s := initialSession()
	s.width, s.height = 100, 40
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.openKeybindsModal()

	// Rebind quit to ctrl+q first.
	s.applyKeybind(0, "ctrl+q")
	if s.keybinds["quit"] != "ctrl+q" {
		t.Fatal("setup: rebind failed")
	}

	// Navigate to quit (index 0) and press backspace to reset.
	s.modal.cursor = 0
	_, _ = s.handleKeybindsKey(tea.KeyMsg{Type: tea.KeyBackspace})
	if s.keybinds["quit"] != "ctrl+c" {
		t.Fatalf("after backspace reset: quit=%q, want ctrl+c", s.keybinds["quit"])
	}
}

// TestKeybindsModalClose verifies the close key closes the modal.
func TestKeybindsModalClose(t *testing.T) {
	s := initialSession()
	s.width, s.height = 100, 40
	s.openKeybindsModal()
	_, _ = s.handleKeybindsKey(keyMsg("esc"))
	if s.modal.kind != modalNone {
		t.Fatalf("esc should close keybinds modal; got kind=%v", s.modal.kind)
	}
}

// TestKeybindsModalRender verifies the modal renders without panic and shows
// the action name and key.
func TestKeybindsModalRender(t *testing.T) {
	s := initialSession()
	s.width, s.height = 100, 40
	s.openKeybindsModal()
	out := s.renderKeybindsModal()
	if out == "" {
		t.Fatal("renderKeybindsModal returned empty string")
	}
	// Should contain the title.
	if !contains(out, "Keybindings") {
		t.Error("renderKeybindsModal should contain 'Keybindings' title")
	}
}

// ---------------------------------------------------------------------------
// Integration: handleKey uses the keymap
// ---------------------------------------------------------------------------

// TestHandleKeyQuitUsesKeymap verifies that the quit action goes through kb().
// After remapping quit to ctrl+q, ctrl+c should no longer quit and ctrl+q
// should.
func TestHandleKeyQuitUsesKeymap(t *testing.T) {
	s := initialSession()
	s.width, s.height = 80, 24
	s.modal = modal{}
	s.coreCmd = nil // so the kill path doesn't crash

	// Remap quit.
	s.keybinds["quit"] = "ctrl+q"

	// ctrl+c should NOT quit (it's no longer the quit binding). With no modal
	// and no other state, it falls through to the input handler. We verify it
	// doesn't produce a tea.Quit command.
	_, cmd := s.handleKey(keyMsg("ctrl+c"))
	if cmd != nil {
		if _, ok := cmd().(tea.QuitMsg); ok {
			t.Fatal("ctrl+c should not quit after remap to ctrl+q")
		}
	}

	// ctrl+q SHOULD quit. We can't easily test tea.Quit return here because the
	// handler kills the core process. Instead verify the key matches.
	if !s.kb(keyMsg("ctrl+q"), "quit") {
		t.Error("ctrl+q should match the remapped quit binding")
	}
}

// TestCommandPaletteOpensViaKeymap verifies ctrl+p and ctrl+k both open the
// palette through the keymap.
func TestCommandPaletteOpensViaKeymap(t *testing.T) {
	s := initialSession()
	s.width, s.height = 80, 24
	s.modal = modal{}
	s.input.SetValue("") // ensure empty input

	_, _ = s.handleKey(keyMsg("ctrl+p"))
	if s.modal.kind != modalCommand {
		t.Fatalf("ctrl+p should open command palette; got %v", s.modal.kind)
	}

	s.modal = modal{}
	_, _ = s.handleKey(keyMsg("ctrl+k"))
	if s.modal.kind != modalCommand {
		t.Fatalf("ctrl+k should open command palette; got %v", s.modal.kind)
	}
}

// TestScrollKeysUseKeymap verifies scrolling keys go through kb().
func TestScrollKeysUseKeymap(t *testing.T) {
	s := initialSession()
	s.width, s.height = 80, 24
	s.viewport.SetContent("line\nline\nline\nline\nline\n")

	// pgup should be consumed by handleScrollKey.
	if !s.handleScrollKey(keyMsg("pgup")) {
		t.Error("pgup should be consumed by handleScrollKey")
	}
	if !s.handleScrollKey(keyMsg("pgdown")) {
		t.Error("pgdown should be consumed by handleScrollKey")
	}
	if !s.handleScrollKey(keyMsg("ctrl+up")) {
		t.Error("ctrl+up should be consumed by handleScrollKey")
	}
	if !s.handleScrollKey(keyMsg("ctrl+home")) {
		t.Error("ctrl+home should be consumed by handleScrollKey")
	}
	// A non-scroll key should not be consumed.
	if s.handleScrollKey(keyMsg("x")) {
		t.Error("'x' should not be consumed by handleScrollKey")
	}
}

// TestApprovalKeysUseKeymap verifies y/a/n and esc go through kb().
func TestApprovalKeysUseKeymap(t *testing.T) {
	s := initialSession()
	s.width, s.height = 80, 24
	s.pendingApproval = &approvalPrompt{requestID: "r1", tool: "bash", args: "{}"}

	// 'y' should approve (sendCore is a no-op without core).
	_, _ = s.handleKey(keyMsg("y"))
	if s.pendingApproval != nil {
		t.Fatal("y should approve and clear pendingApproval")
	}

	// Test deny with 'n'.
	s.pendingApproval = &approvalPrompt{requestID: "r2", tool: "bash", args: "{}"}
	_, _ = s.handleKey(keyMsg("n"))
	if s.pendingApproval != nil {
		t.Fatal("n should deny and clear pendingApproval")
	}

	// Test deny with esc (close).
	s.pendingApproval = &approvalPrompt{requestID: "r3", tool: "bash", args: "{}"}
	_, _ = s.handleKey(keyMsg("esc"))
	if s.pendingApproval != nil {
		t.Fatal("esc should deny and clear pendingApproval")
	}

	// Test uppercase Y.
	s.pendingApproval = &approvalPrompt{requestID: "r4", tool: "bash", args: "{}"}
	_, _ = s.handleKey(keyMsg("Y"))
	if s.pendingApproval != nil {
		t.Fatal("Y (uppercase) should approve and clear pendingApproval")
	}
}

// TestRemappedApprovalKey verifies a remapped approve key works.
func TestRemappedApprovalKey(t *testing.T) {
	s := initialSession()
	s.width, s.height = 80, 24
	s.keybinds["approve"] = "x"
	s.pendingApproval = &approvalPrompt{requestID: "r1", tool: "bash", args: "{}"}

	// 'y' should no longer approve.
	_, _ = s.handleKey(keyMsg("y"))
	if s.pendingApproval == nil {
		t.Fatal("y should NOT approve after remap to x")
	}
	// 'x' should approve.
	_, _ = s.handleKey(keyMsg("x"))
	if s.pendingApproval != nil {
		t.Fatal("x should approve after remap")
	}
}

// TestHelpTextUsesKeymap verifies the help text reflects remapped keys.
func TestHelpTextUsesKeymap(t *testing.T) {
	s := initialSession()
	s.keybinds["quit"] = "ctrl+q"
	text := s.helpText()
	if !contains(text, "Ctrl+Q") {
		t.Error("helpText should show remapped quit key Ctrl+Q")
	}
	if !contains(text, "Ctrl+P") {
		t.Error("helpText should show default command_palette key Ctrl+P")
	}
}

// TestSettingsPersistenceRoundTrip verifies keybind overrides survive a
// save/load cycle. Uses a temp file so the user's real settings are untouched.
func TestSettingsPersistenceRoundTrip(t *testing.T) {
	s := initialSession()
	// Redirect settings to a temp file to avoid clobbering real prefs.
	tmp := t.TempDir()
	s.settings.path = tmp + "/settings.json"
	s.applyKeybind(0, "ctrl+q") // rebind quit
	if s.settings.Keybinds["quit"] != "ctrl+q" {
		t.Fatal("settings.Keybinds should have the override before save")
	}
	err := s.settings.save()
	if err != nil {
		t.Fatalf("save failed: %v", err)
	}
	// Reload from the temp path.
	disk := &settingsStore{path: s.settings.path}
	data, err := os.ReadFile(disk.path)
	if err != nil {
		t.Fatalf("read temp settings: %v", err)
	}
	var onDisk settingsStore
	if err := json.Unmarshal(data, &onDisk); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	reloaded := &settingsStore{path: disk.path, Keybinds: onDisk.Keybinds}
	if reloaded.Keybinds["quit"] != "ctrl+q" {
		t.Fatalf("after reload: Keybinds[quit]=%q, want ctrl+q", reloaded.Keybinds["quit"])
	}
	// effectiveKeybinds should merge correctly.
	eff := effectiveKeybinds(reloaded.Keybinds)
	if eff["quit"] != "ctrl+q" {
		t.Fatalf("effective quit=%q, want ctrl+q", eff["quit"])
	}
	if eff["close"] != "esc" {
		t.Fatalf("effective close=%q, want esc (default)", eff["close"])
	}
}

// ---------------------------------------------------------------------------
// TestCtrlEnterRejectedForNonSteer verifies that binding ctrl+enter to a
// non-steer action is rejected (it can only ever fire for steer via the CSI
// dispatch, so any other binding would be silently dead).
func TestCtrlEnterRejectedForNonSteer(t *testing.T) {
	s := initialSession()
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.openKeybindsModal()
	// Cursor at 0 = quit (not steer).
	s.modal.editing = true
	s.captureKeybind("ctrl+enter")
	if s.modal.editing {
		t.Error("captureKeybind(ctrl+enter) for non-steer should cancel capture")
	}
	if s.keybinds["quit"] == "ctrl+enter" {
		t.Error("ctrl+enter should not be bound to quit")
	}
}

// TestCtrlEnterAcceptedForSteer verifies ctrl+enter CAN be bound to steer.
func TestCtrlEnterAcceptedForSteer(t *testing.T) {
	s := initialSession()
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.openKeybindsModal()
	// Find steer index.
	steerIdx := -1
	for i, d := range keybindDefs {
		if d.Action == "steer" {
			steerIdx = i
			break
		}
	}
	if steerIdx < 0 {
		t.Fatal("steer action not found")
	}
	s.modal.cursor = steerIdx
	s.modal.editing = true
	s.captureKeybind("ctrl+enter")
	if s.keybinds["steer"] != "ctrl+enter" {
		t.Fatalf("steer should be ctrl+enter; got %q", s.keybinds["steer"])
	}
}

// TestEscGuaranteedCloseInKeybindsModal verifies esc closes the keybinds modal
// even when close is remapped to something else.
func TestEscGuaranteedCloseInKeybindsModal(t *testing.T) {
	s := initialSession()
	s.width, s.height = 100, 40
	s.openKeybindsModal()
	// Remap close to ctrl+g so esc is no longer the close binding.
	s.keybinds["close"] = "ctrl+g"
	// Esc should STILL close the modal (guaranteed escape hatch).
	_, _ = s.handleKeybindsKey(keyMsg("esc"))
	if s.modal.kind != modalNone {
		t.Fatal("esc should always close the keybinds modal (guaranteed escape)")
	}
}

// TestCaseInsensitiveOnlyApproval verifies that single-char case-insensitivity
// applies to approval keys but NOT to vim-style nav alts (j/k/h/l).
func TestCaseInsensitiveOnlyApproval(t *testing.T) {
	s := initialSession()
	s.keybinds = defaultKeybinds() // ensure clean defaults regardless of on-disk settings
	// Approval: y and Y both match.
	if !s.kb(keyMsg("y"), "approve") {
		t.Error("y should match approve")
	}
	if !s.kb(keyMsg("Y"), "approve") {
		t.Error("Y should match approve (case-insensitive)")
	}
	// Nav alt: k matches, K does NOT (exact match for non-approval).
	if !s.kb(keyMsg("k"), "nav_up_alt") {
		t.Error("k should match nav_up_alt")
	}
	if s.kb(keyMsg("K"), "nav_up_alt") {
		t.Error("K should NOT match nav_up_alt (only approval keys are case-insensitive)")
	}
}

// TestClearKeybind verifies that clearing a binding sets it to empty (disabled)
// so kb never matches it, and that the empty value persists through save/load.
func TestClearKeybind(t *testing.T) {
	s := initialSession()
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.keybinds = defaultKeybinds()

	// Find command_palette_alt (ctrl+k) index.
	altIdx := -1
	for i, d := range keybindDefs {
		if d.Action == "command_palette_alt" {
			altIdx = i
			break
		}
	}
	if altIdx < 0 {
		t.Fatal("command_palette_alt not found in keybindDefs")
	}

	// Before clearing: ctrl+k matches command_palette_alt.
	if !s.kb(keyMsg("ctrl+k"), "command_palette_alt") {
		t.Fatal("ctrl+k should match command_palette_alt before clearing")
	}

	// Clear it.
	s.clearKeybind(altIdx)
	if s.keybinds["command_palette_alt"] != "" {
		t.Fatalf("after clear: command_palette_alt=%q, want empty", s.keybinds["command_palette_alt"])
	}
	if s.settings.Keybinds["command_palette_alt"] != "" {
		t.Fatalf("persisted override should be empty: got %q", s.settings.Keybinds["command_palette_alt"])
	}

	// After clearing: ctrl+k no longer matches command_palette_alt.
	if s.kb(keyMsg("ctrl+k"), "command_palette_alt") {
		t.Error("ctrl+k should NOT match command_palette_alt after clearing (disabled)")
	}
	// But ctrl+p (the primary) still works.
	if !s.kb(keyMsg("ctrl+p"), "command_palette") {
		t.Error("ctrl+p should still match command_palette (only alt was cleared)")
	}
}

// TestClearKeybindPersists verifies the empty (disabled) value survives a
// save/load cycle — it must NOT be dropped or treated as "fall back to default".
func TestClearKeybindPersists(t *testing.T) {
	s := initialSession()
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.keybinds = defaultKeybinds()

	// Clear nav_up_alt (k).
	altIdx := -1
	for i, d := range keybindDefs {
		if d.Action == "nav_up_alt" {
			altIdx = i
			break
		}
	}
	if altIdx < 0 {
		t.Fatal("nav_up_alt not found")
	}
	s.clearKeybind(altIdx)
	_ = s.settings.save()

	// Reload from the temp file (loadSettings reads the real path, not our temp).
	data, err := os.ReadFile(s.settings.path)
	if err != nil {
		t.Fatalf("read temp settings: %v", err)
	}
	var onDisk settingsStore
	if err := json.Unmarshal(data, &onDisk); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if onDisk.Keybinds["nav_up_alt"] != "" {
		t.Fatalf("persisted nav_up_alt should be empty (disabled): got %q", onDisk.Keybinds["nav_up_alt"])
	}

	// effectiveKeybinds must keep it empty (disabled), not fall back to default "k".
	eff := effectiveKeybinds(onDisk.Keybinds)
	if eff["nav_up_alt"] != "" {
		t.Fatalf("effective nav_up_alt should be empty (disabled): got %q, want empty", eff["nav_up_alt"])
	}
	// kb should return false for the disabled action.
	s2 := initialSession()
	s2.keybinds = eff
	if s2.kb(keyMsg("k"), "nav_up_alt") {
		t.Error("kb should return false for a disabled (empty) binding")
	}
}

// TestKeybindsModalClearViaDelete verifies the Delete key clears a binding in
// the /keybinds modal.
func TestKeybindsModalClearViaDelete(t *testing.T) {
	s := initialSession()
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.keybinds = defaultKeybinds()
	s.openKeybindsModal()

	// Navigate to command_palette_alt and clear it via Delete.
	// command_palette_alt is at index 3 (Global group: quit, toggle_reasoning,
	// toggle_tool_output, command_palette, command_palette_alt).
	for i, d := range keybindDefs {
		if d.Action == "command_palette_alt" {
			s.modal.cursor = i
			break
		}
	}
	_, _ = s.handleKeybindsKey(tea.KeyMsg{Type: tea.KeyDelete})
	if s.keybinds["command_palette_alt"] != "" {
		t.Fatalf("after Delete: command_palette_alt=%q, want empty", s.keybinds["command_palette_alt"])
	}
}

// TestKeybindsModalResetThenClear verifies Backspace (reset to default) and
// Delete (clear/disable) are distinct operations.
func TestKeybindsModalResetThenClear(t *testing.T) {
	s := initialSession()
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.keybinds = defaultKeybinds()

	// Rebind quit to ctrl+q.
	s.applyKeybind(0, "ctrl+q")
	if s.keybinds["quit"] != "ctrl+q" {
		t.Fatal("setup failed")
	}

	// Backspace resets to default (ctrl+c).
	s.openKeybindsModal()
	s.modal.cursor = 0
	_, _ = s.handleKeybindsKey(tea.KeyMsg{Type: tea.KeyBackspace})
	if s.keybinds["quit"] != "ctrl+c" {
		t.Fatalf("after Backspace (reset): quit=%q, want ctrl+c", s.keybinds["quit"])
	}

	// Delete clears (disables) — sets to empty.
	_, _ = s.handleKeybindsKey(tea.KeyMsg{Type: tea.KeyDelete})
	if s.keybinds["quit"] != "" {
		t.Fatalf("after Delete (clear): quit=%q, want empty", s.keybinds["quit"])
	}
}

// TestKeybindLabelDisabled verifies keybindLabel renders disabled (empty) as "—".
func TestKeybindLabelDisabled(t *testing.T) {
	if got := keybindLabel(""); got != "—" {
		t.Fatalf("keybindLabel(\"\") = %q, want —", got)
	}
}

// helpers
// ---------------------------------------------------------------------------

func contains(s, substr string) bool {
	return len(s) >= len(substr) && (indexOf(s, substr) >= 0)
}

func indexOf(s, substr string) int {
	for i := 0; i <= len(s)-len(substr); i++ {
		if s[i:i+len(substr)] == substr {
			return i
		}
	}
	return -1
}
