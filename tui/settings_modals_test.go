package main

import (
	"encoding/json"
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
			if items[i].meta != mode {
				t.Errorf("%s: items[%d] mode=%q, want %q", cmd, i, items[i].meta, mode)
			}
		}
		body := stripANSI(s.renderModalBody())
		if !strings.Contains(body, "Approval Mode") {
			t.Errorf("%s: modal body missing title:\n%s", cmd, body)
		}
		// Selected mode should appear (cursor starts on current = destructive).
		if !strings.Contains(body, "Ask for destructive tools") {
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

// TestApprovalEscalationDoesNotResetMode: core's "<kind>:always" event must not
// flip the footer/settings to "destructive" (normalizeApproval's fallback).
func TestApprovalEscalationDoesNotResetMode(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.applyApprovalMode("never")
	if s.approvalMode() != "never" {
		t.Fatalf("precondition: approvalMode=%q", s.approvalMode())
	}

	raw, _ := json.Marshal(map[string]any{
		"type": "approval_changed",
		"mode": "destructive:always",
	})
	s.handleCoreEvent(&coreEvent{Type: "approval_changed", Raw: raw})

	if s.approvalMode() != "never" {
		t.Fatalf("after escalation: approvalMode=%q, want never", s.approvalMode())
	}
	if s.settings.Approval != "never" {
		t.Fatalf("settings.Approval=%q, want never", s.settings.Approval)
	}
	// Stale escalation string must not leak into the display path.
	s.approvalModeStr = "destructive:always"
	if s.approvalMode() != "never" {
		t.Fatalf("approvalMode with leaked escalation=%q, want never (from settings)", s.approvalMode())
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
	for _, want := range []string{"/approval", "/reasoning", "/theme", "/bash-timeout", "/sandbox"} {
		if !strings.Contains(body, want) {
			t.Errorf("settings hub missing %s:\n%s", want, body)
		}
	}
	if strings.Contains(body, "/mouse-wheel") {
		t.Errorf("always-on mouse interaction should not have an opt-in setting:\n%s", body)
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
		{"/reasoning", modalReasoning},
		{"/theme", modalTheme},
		{"/bash-timeout", modalValueEdit},
		{"/auto-compact", modalAutoCompact},
		{"/sandbox", modalSandbox},
		{"/no-network", modalNoNetwork},
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

	// The retired toggle remains a harmless compatibility alias for scripts,
	// but no longer changes state or opens a modal.
	s.handleUserLine("/mouse-wheel off")
	if s.modal.kind != modalNone {
		t.Error("retired mouse-wheel command should not open a modal")
	}

	s.handleUserLine("/sandbox firejail")
	// Deprecated backends migrate to microsandbox and start the fail-closed
	// enable flow (status check). The setting is NOT persisted until the core
	// reports the environment ready.
	if s.settings.Sandbox != "none" {
		t.Errorf("sandbox = %q, want none (not persisted until ready)", s.settings.Sandbox)
	}
	if !s.pendingSandboxEnable {
		t.Error("/sandbox firejail should set pendingSandboxEnable")
	}
	if s.modal.kind != modalSandboxStatus {
		t.Errorf("modal = %v, want modalSandboxStatus", s.modal.kind)
	}

	s.handleUserLine("/auto-compact off")
	if s.coreAutoCompact {
		t.Error("auto-compact off should set coreAutoCompact=false")
	}
	if s.settings.AutoCompact {
		t.Error("auto-compact off should persist AutoCompact=false")
	}

	s.handleUserLine("/bash-timeout 90")
	if s.coreBashTimeout != 90 {
		t.Errorf("bash timeout = %d, want 90", s.coreBashTimeout)
	}
	if s.settings.BashTimeoutSecs != 90 {
		t.Errorf("settings.BashTimeoutSecs = %d, want 90", s.settings.BashTimeoutSecs)
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
