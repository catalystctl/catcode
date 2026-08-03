package main

import (
	"encoding/json"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

// pluginTrustPromptEvent builds a real `plugin_trust_prompt` core event so the
// test exercises the actual event→modal wiring (not direct field assignment).
func pluginTrustPromptEvent(t *testing.T, plugins string) *coreEvent {
	t.Helper()
	raw, err := json.Marshal(map[string]any{
		"plugins": json.RawMessage(plugins),
	})
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	return &coreEvent{Type: "plugin_trust_prompt", Raw: raw}
}

func TestPluginTrustPromptOpensModal(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.layout()
	s.handleCoreEvent(pluginTrustPromptEvent(t,
		`[{"name":"shady","version":"1.0.0","description":"Hook-heavy linter","path":"/ws/.catalyst-code/plugins/shady","decision":""}]`))
	if s.modal.kind != modalPluginTrust {
		t.Fatalf("kind=%v, want modalPluginTrust", s.modal.kind)
	}
	if s.pluginTrust == nil || len(s.pluginTrust.entries) != 1 {
		t.Fatalf("pluginTrust=%+v, want 1 entry", s.pluginTrust)
	}
	if s.pluginTrust.decisions["shady"] != "" {
		t.Fatalf("initial decision=%q, want undecided", s.pluginTrust.decisions["shady"])
	}
	// Render must include the plugin name and the apply row.
	view := stripANSI(s.renderPluginTrustModal())
	if !strings.Contains(view, "shady") || !strings.Contains(view, "Apply & close") {
		t.Fatalf("render missing content: %q", view)
	}
}

func TestPluginTrustCycleAndApply(t *testing.T) {
	s := initialSession()
	s.keybinds = defaultKeybinds() // isolate from user settings
	s.ready = true
	s.width, s.height = 80, 24
	s.layout()
	s.handleCoreEvent(pluginTrustPromptEvent(t,
		`[{"name":"shady","version":"1.0.0","description":"x","path":"/p","decision":""},{"name":"linter","version":"2.0.0","description":"y","path":"/p","decision":"deny"}]`))
	if s.modal.kind != modalPluginTrust {
		t.Fatalf("kind=%v, want modalPluginTrust", s.modal.kind)
	}

	// Focus row 0 (shady) and cycle: undecided → trust.
	s.modal.cursor = 0
	s.handlePluginTrustKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if got := s.pluginTrust.decisions["shady"]; got != "trust" {
		t.Fatalf("after enter: shady=%q, want trust", got)
	}
	// Cycle again: trust → deny.
	s.handlePluginTrustKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if got := s.pluginTrust.decisions["shady"]; got != "deny" {
		t.Fatalf("after second enter: shady=%q, want deny", got)
	}

	// Space on row 1 (linter, was deny) → deny → undecided.
	s.modal.cursor = 1
	s.handlePluginTrustKey(tea.KeyPressMsg{Code: tea.KeySpace})
	if got := s.pluginTrust.decisions["linter"]; got != "" {
		t.Fatalf("after space: linter=%q, want undecided", got)
	}
	// Space again: undecided → trust.
	s.handlePluginTrustKey(tea.KeyPressMsg{Code: tea.KeySpace})
	if got := s.pluginTrust.decisions["linter"]; got != "trust" {
		t.Fatalf("after second space: linter=%q, want trust", got)
	}

	// Apply & close: only decided entries are sent to the core.
	cw := &captureWriter{}
	s.coreIn = cw
	s.modal.cursor = s.pluginTrustRows() - 1 // apply row
	s.handlePluginTrustKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.modal.kind != modalNone {
		t.Fatalf("modal still open after apply: kind=%v", s.modal.kind)
	}
	if len(cw.lines) != 1 {
		t.Fatalf("expected one command sent, got %d", len(cw.lines))
	}
	var sent map[string]any
	if err := json.Unmarshal([]byte(cw.lines[0]), &sent); err != nil {
		t.Fatalf("unmarshal sent command: %v", err)
	}
	if sent["type"] != "plugin_trust_decisions" {
		t.Fatalf("expected plugin_trust_decisions sent, got %+v", sent)
	}
	decisions, ok := sent["decisions"].(map[string]any)
	if !ok {
		t.Fatalf("decisions type=%T", sent["decisions"])
	}
	if decisions["shady"] != "deny" || decisions["linter"] != "trust" {
		t.Fatalf("decisions=%v", decisions)
	}
}

func TestPluginTrustEmptyPrompt(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.layout()
	s.handleCoreEvent(pluginTrustPromptEvent(t, `[]`))
	if s.modal.kind != modalNone {
		t.Fatalf("empty prompt must not open a modal, kind=%v", s.modal.kind)
	}
}

func TestPluginTrustSlashCommand(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	cw := &captureWriter{}
	s.coreIn = cw
	s.handleUserLine("/plugin-trust")
	if len(cw.lines) != 1 {
		t.Fatalf("expected one command sent, got %d", len(cw.lines))
	}
	var sent map[string]any
	if err := json.Unmarshal([]byte(cw.lines[0]), &sent); err != nil {
		t.Fatalf("unmarshal sent command: %v", err)
	}
	if sent["type"] != "plugin_trust_prompt" {
		t.Fatalf("expected plugin_trust_prompt sent, got %+v", sent)
	}
}
