package main

import (
	"encoding/json"
	"runtime"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

func TestSandboxModesMatchHostPlatform(t *testing.T) {
	if !sandboxModeAvailable("none") {
		t.Fatal("none sandbox must always be available")
	}
	if got, want := sandboxModeAvailable("firejail"), runtime.GOOS == "linux"; got != want {
		t.Fatalf("firejail available = %v, want %v on %s", got, want, runtime.GOOS)
	}
	if got, want := sandboxModeAvailable("seatbelt"), runtime.GOOS == "darwin"; got != want {
		t.Fatalf("seatbelt available = %v, want %v on %s", got, want, runtime.GOOS)
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
