package main

import (
	"encoding/json"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

func TestSandboxNormalizationMigratesLegacyBackends(t *testing.T) {
	// Microsandbox is cross-platform (Linux KVM · Apple Silicon · Windows WHP);
	// the selector is never gated on the host OS, so these are pure value tests.
	cases := []struct {
		in       string
		wantMode string
		wantDep  bool
	}{
		{"none", "none", false},
		{"off", "none", false},
		{"", "none", false},
		{"microsandbox", "microsandbox", false},
		{"msb", "microsandbox", false},
		{"on", "microsandbox", false},
		// Deprecated backends must preserve the intent to sandbox (never none).
		{"firejail", "microsandbox", true},
		{"fj", "microsandbox", true},
		{"seatbelt", "microsandbox", true},
		{"macos", "microsandbox", true},
		{"sandbox-exec", "microsandbox", true},
		// Unknown token is rejected (returns ""), not silently coerced to none.
		{"bogus", "", false},
	}
	for _, tc := range cases {
		mode, dep := normalizeSandboxValue(tc.in)
		if mode != tc.wantMode || dep != tc.wantDep {
			t.Errorf("normalizeSandboxValue(%q) = (%q,%v), want (%q,%v)", tc.in, mode, dep, tc.wantMode, tc.wantDep)
		}
	}
}

func TestSandboxSelectorOnlyExposesNoneAndMicrosandbox(t *testing.T) {
	s := initialSession()
	items := s.sandboxItems()
	if len(items) != 2 {
		t.Fatalf("sandbox items len=%d, want 2 (none · microsandbox): %#v", len(items), items)
	}
	for _, it := range items {
		if it.meta != "none" && it.meta != "microsandbox" {
			t.Errorf("sandbox item %q is not none/microsandbox", it.label)
		}
		// No legacy backend names may leak into the selector.
		body := strings.ToLower(it.label + " " + it.desc)
		for _, bad := range []string{"firejail", "seatbelt", "sandbox-exec"} {
			if strings.Contains(body, bad) {
				t.Errorf("sandbox item leaks legacy backend %q: %s", bad, body)
			}
		}
	}
}

func TestPendingApprovalDiffExpandsAndUsesLiveBindings(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.keybinds["approve"] = "z"
	s.keybinds["toggle_tool_output"] = "ctrl+e"
	s.pendingApproval = &approvalPrompt{
		tool: "edit",
		args: `{"path":"main.go"}`,
		diff: strings.Join([]string{
			"--- a/main.go", "+++ b/main.go", "@@ -1 +1 @@", "-old", "+new", "+two", "+three",
		}, "\n"),
	}

	collapsed := stripANSI(s.renderApprovalBanner())
	if !strings.Contains(collapsed, "[Z] once") || !strings.Contains(collapsed, "Ctrl+E expand") {
		t.Fatalf("approval hints do not reflect keymap:\n%s", collapsed)
	}
	_, _ = s.handleKey(tea.KeyPressMsg{Code: 'e', Mod: tea.ModCtrl})
	if !s.pendingApproval.expanded {
		t.Fatal("pending approval diff did not expand")
	}
	expanded := stripANSI(s.renderApprovalBanner())
	if !strings.Contains(expanded, "+three") {
		t.Fatalf("expanded approval omitted tail:\n%s", expanded)
	}
}

func TestAsyncPickerCancelIgnoresLatePluginReply(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.requestPluginPicker(pluginModeToggle)
	if s.modal.kind != modalPlugins || !s.modal.loading || !s.pendingPluginPicker {
		t.Fatalf("picker did not enter loading state: modal=%v loading=%v pending=%v", s.modal.kind, s.modal.loading, s.pendingPluginPicker)
	}
	s.closeModal()
	raw, _ := json.Marshal(map[string]any{"type": "plugins_list", "plugins": []any{}})
	s.handleCoreEvent(&coreEvent{Type: "plugins_list", Raw: raw})
	if s.modal.kind != modalNone {
		t.Fatalf("late reply reopened cancelled picker: %v", s.modal.kind)
	}
}

func TestRecentCommandsLeadPalette(t *testing.T) {
	s := initialSession()
	s.recordRecentCommand("/theme")
	s.recordRecentCommand("/model")
	items := s.commandItems()
	if len(items) < 2 || items[0].group != "Recent" || items[0].label != "/model" || items[1].label != "/theme" {
		t.Fatalf("recent commands not palette-first: %#v", items[:min(3, len(items))])
	}
}
