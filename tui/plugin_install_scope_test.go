package main

import (
	"testing"

	tea "charm.land/bubbletea/v2"
)

func TestParsePluginInstallArgs(t *testing.T) {
	cases := []struct {
		args      []string
		wantPath  string
		wantScope string
		wantErr   bool
	}{
		// No scope → empty (UI prompts global vs workspace).
		{[]string{"/tmp/p"}, "/tmp/p", "", false},
		{[]string{"/tmp/p", "workspace"}, "/tmp/p", "workspace", false},
		{[]string{"--workspace", "owner/repo"}, "owner/repo", "workspace", false},
		{[]string{"https://github.com/a/b", "--global"}, "https://github.com/a/b", "global", false},
		{[]string{"-w", "./plugin"}, "./plugin", "workspace", false},
		{[]string{"-g", "karutoil/catcode-chatgpt-provider"}, "karutoil/catcode-chatgpt-provider", "global", false},
		{[]string{}, "", "", true},
		{[]string{"a", "b"}, "", "", true},
		{[]string{"workspace"}, "", "", true}, // scope alone is not a source
	}
	for _, tc := range cases {
		path, scope, err := parsePluginInstallArgs(tc.args)
		if tc.wantErr {
			if err == nil {
				t.Fatalf("args %v: expected error", tc.args)
			}
			continue
		}
		if err != nil {
			t.Fatalf("args %v: unexpected error: %v", tc.args, err)
		}
		if path != tc.wantPath || scope != tc.wantScope {
			t.Fatalf("args %v: got path=%q scope=%q, want path=%q scope=%q",
				tc.args, path, scope, tc.wantPath, tc.wantScope)
		}
	}
}

func TestPluginInstallOpensScopePickerWhenScopeOmitted(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.handleUserLine("/plugin-install https://github.com/karutoil/catcode-chatgpt-provider")
	if s.modal.kind != modalPluginInstallScope {
		t.Fatalf("kind=%v, want modalPluginInstallScope", s.modal.kind)
	}
	if s.pendingPluginInstallPath != "https://github.com/karutoil/catcode-chatgpt-provider" {
		t.Fatalf("pending path=%q", s.pendingPluginInstallPath)
	}
	items := s.pluginInstallScopeItems()
	if len(items) != 2 || items[0].label != "global" || items[1].label != "workspace" {
		t.Fatalf("scope items=%v", items)
	}
}

func TestPluginInstallSkipsScopePickerWhenScopeGiven(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.handleUserLine("/plugin-install ./plug workspace")
	if s.modal.kind == modalPluginInstallScope {
		t.Fatalf("should not open scope picker when scope is explicit")
	}
	if s.pendingPluginInstallPath != "" {
		t.Fatalf("pending path should be empty, got %q", s.pendingPluginInstallPath)
	}
}

func TestPluginInstallModalThenScopePicker(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.handleUserLine("/plugin-install")
	if s.modal.kind != modalValueEdit || s.modal.editTarget != editTargetPluginInstall {
		t.Fatalf("bare /plugin-install: kind=%v target=%q", s.modal.kind, s.modal.editTarget)
	}
	s.modal.editBuf.SetValue("owner/repo")
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.modal.kind != modalPluginInstallScope {
		t.Fatalf("after path: kind=%v, want modalPluginInstallScope", s.modal.kind)
	}
	if s.pendingPluginInstallPath != "owner/repo" {
		t.Fatalf("pending=%q", s.pendingPluginInstallPath)
	}
}

