package main

import (
	"path/filepath"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

// TestApprovalCommandOpensPicker: bare /approval (and /approvals) must open the
// dedicated approval modal, not the old multi-field settings editor.
func TestApprovalCommandOpensPicker(t *testing.T) {
	for _, cmd := range []string{"/approval", "/approvals"} {
		s := initialSession()
		s.ready = true
		s.width, s.height = 100, 40
		s.approvalModeStr = "destructive"
		s.handleUserLine(cmd)
		if s.modal.kind != modalApproval {
			t.Fatalf("%s: modal kind = %v, want modalApproval", cmd, s.modal.kind)
		}
		items := s.approvalItems()
		if len(items) != 3 {
			t.Fatalf("%s: approvalItems len=%d, want 3", cmd, len(items))
		}
		for i, mode := range []string{"never", "destructive", "always"} {
			if items[i].label != mode {
				t.Errorf("%s: items[%d]=%q, want %q", cmd, i, items[i].label, mode)
			}
		}
		body := stripANSI(s.renderModalBody())
		if !strings.Contains(body, "Approval Mode") {
			t.Errorf("%s: modal body missing title:\n%s", cmd, body)
		}
		// Selected mode should appear (cursor starts on current = destructive).
		if !strings.Contains(body, "destructive") && !strings.Contains(body, "destru") {
			t.Errorf("%s: modal body missing current mode:\n%s", cmd, body)
		}
	}
}

// TestApprovalCommandWithArgSetsDirectly: /approval always applies without a modal.
func TestApprovalCommandWithArgSetsDirectly(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.handleUserLine("/approval always")
	if s.modal.kind != modalNone {
		t.Fatalf("expected no modal after /approval always, got %v", s.modal.kind)
	}
	if s.settings.Approval != "always" {
		t.Errorf("settings.Approval = %q, want always", s.settings.Approval)
	}
}

// TestSettingsHubOpensDedicatedModals: /settings is a hub; selecting each
// entry opens the corresponding dedicated modal (not a field editor).
func TestSettingsHubOpensDedicatedModals(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 100, 40
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.approvalModeStr = "destructive"
	s.coreBashTimeout = 30
	s.settings.IdleTimeout = 120

	s.handleUserLine("/settings")
	if s.modal.kind != modalSettings {
		t.Fatalf("modal kind = %v, want modalSettings", s.modal.kind)
	}
	body := stripANSI(s.renderModalBody())
	for _, want := range []string{"/approval", "/reasoning", "/theme", "/bash-timeout", "/sandbox", "/mouse-wheel"} {
		if !strings.Contains(body, want) {
			t.Errorf("settings hub missing %s:\n%s", want, body)
		}
	}
	// Must not show the old field-editor chrome.
	if strings.Contains(body, "enter edit/apply") || strings.Contains(body, "←→ cycle") {
		t.Errorf("settings hub still looks like the old field editor:\n%s", body)
	}

	// Select /approval from the hub.
	items := s.settingsHubItems()
	approvalIdx := -1
	for i, it := range items {
		if it.label == "/approval" {
			approvalIdx = i
			break
		}
	}
	if approvalIdx < 0 {
		t.Fatal("/approval not in settings hub")
	}
	s.modal.cursor = approvalIdx
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.modal.kind != modalApproval {
		t.Fatalf("after select /approval: kind = %v, want modalApproval", s.modal.kind)
	}
}

// TestDedicatedSettingCommandsOpenModals covers bare slash commands for each
// former settings field.
func TestDedicatedSettingCommandsOpenModals(t *testing.T) {
	cases := []struct {
		cmd  string
		kind modalKind
	}{
		{"/key", modalValueEdit},
		{"/reasoning", modalReasoning},
		{"/theme", modalTheme},
		{"/bash-timeout", modalValueEdit},
		{"/auto-compact", modalAutoCompact},
		{"/sandbox", modalSandbox},
		{"/no-network", modalNoNetwork},
		{"/mouse-wheel", modalMouseWheel},
		{"/idle-timeout", modalValueEdit},
		{"/max-session-tokens", modalValueEdit},
		{"/settings", modalSettings},
	}
	for _, tc := range cases {
		s := initialSession()
		s.ready = true
		s.settings.path = filepath.Join(t.TempDir(), "settings.json")
		s.handleUserLine(tc.cmd)
		if s.modal.kind != tc.kind {
			t.Errorf("%s: kind = %v, want %v", tc.cmd, s.modal.kind, tc.kind)
		}
	}
}

// TestToggleCommandsWithArgs apply without opening a modal.
func TestToggleCommandsWithArgs(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")

	s.handleUserLine("/mouse-wheel on")
	if !s.settings.MouseWheel {
		t.Error("mouse-wheel on should set MouseWheel=true")
	}
	if s.modal.kind != modalNone {
		t.Error("mouse-wheel on should not open a modal")
	}

	s.handleUserLine("/sandbox firejail")
	if s.settings.Sandbox != "firejail" {
		t.Errorf("sandbox = %q, want firejail", s.settings.Sandbox)
	}

	s.handleUserLine("/auto-compact off")
	if s.coreAutoCompact {
		t.Error("auto-compact off should set coreAutoCompact=false")
	}

	s.handleUserLine("/bash-timeout 90")
	if s.coreBashTimeout != 90 {
		t.Errorf("bash timeout = %d, want 90", s.coreBashTimeout)
	}
}

// TestApprovalPickerSelectAppliesMode: enter on a mode in the approval modal
// persists it and closes the modal.
func TestApprovalPickerSelectAppliesMode(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.approvalModeStr = "destructive"
	s.openApprovalPicker()

	// Move to "always" (index 2) and select.
	s.modal.cursor = 2
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.modal.kind != modalNone {
		t.Fatalf("modal should close after select, kind=%v", s.modal.kind)
	}
	if s.settings.Approval != "always" {
		t.Errorf("settings.Approval = %q, want always", s.settings.Approval)
	}
}

// TestValueEditAPIKeyCommit: typing a key in the /key modal and pressing Enter
// scopes it to the active provider.
func TestValueEditAPIKeyCommit(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.activeProvider = "umans"
	s.openAPIKeyModal()
	if s.modal.kind != modalValueEdit || s.modal.editTarget != editTargetAPIKey {
		t.Fatalf("expected API key value-edit modal, got kind=%v target=%q", s.modal.kind, s.modal.editTarget)
	}
	s.modal.editBuf.SetValue("sk-test-from-modal")
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.modal.kind != modalNone {
		t.Fatalf("modal should close, kind=%v", s.modal.kind)
	}
	if got := s.settings.ProviderKeys["umans"]; got != "sk-test-from-modal" {
		t.Errorf("ProviderKeys[umans]=%q, want sk-test-from-modal", got)
	}
}

// TestCommandPaletteApprovalOpensPicker: palette entry for /approval must not
// fall back to the old settings field editor.
func TestCommandPaletteApprovalOpensPicker(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.openCommandPalette()
	// Find /approval in the palette and select it.
	items := s.commandItems()
	idx := -1
	for i, it := range items {
		if it.label == "/approval" {
			idx = i
			break
		}
	}
	if idx < 0 {
		t.Fatal("/approval missing from command palette")
	}
	// runCommandByIndex expects absolute index into commandItems (not filtered).
	s.closeModal()
	cmd := s.runCommandByIndex(idx)
	_ = cmd
	if s.modal.kind != modalApproval {
		t.Fatalf("palette /approval → kind=%v, want modalApproval", s.modal.kind)
	}
}
