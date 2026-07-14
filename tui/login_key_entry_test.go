package main

import (
	"encoding/json"
	"path/filepath"
	"testing"

	tea "charm.land/bubbletea/v2"
)

// TestLoginKeyEntryEnterCommits reproduces the reported bug surface: after
// picking a provider preset in /login and typing a key, pressing Enter should
// commit (close the modal + record the key).
func TestLoginKeyEntryEnterCommits(t *testing.T) {
	s := initialSession()
	s.keybinds = defaultKeybinds()
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.ready = true
	s.width, s.height = 80, 24

	// A first-party preset with no key and no OAuth → prompts for inline key.
	s.providerPresets = []providerPreset{{
		ID:       "umans",
		Label:    "Umans",
		Kind:     "openai",
		BaseURL:  "https://example.com/v1",
		EnvVar:   "UMANS_API_KEY",
		HasKey:   false,
		LoggedIn: false,
	}}

	s.openLoginPicker()
	if s.modal.kind != modalProviders {
		t.Fatalf("expected modalProviders, got %v", s.modal.kind)
	}

	// Select the preset (Enter on the first list item).
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.modal.editing != true {
		t.Fatalf("expected editing=true after picking preset, got false (pendingLogin=%q)", s.pendingLogin)
	}
	if s.pendingLogin != "umans" {
		t.Fatalf("expected pendingLogin=umans, got %q", s.pendingLogin)
	}

	for _, r := range "sk-test-key-1234" {
		s.handleModalKey(tea.KeyPressMsg{Code: r, Text: string(r)})
	}
	if got := s.modal.editBuf.Value(); got != "sk-test-key-1234" {
		t.Fatalf("editBuf value after typing = %q, want sk-test-key-1234", got)
	}

	// Press Enter to commit.
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter})

	if s.modal.kind != modalNone {
		t.Errorf("modal should be closed after Enter; kind=%v editing=%v", s.modal.kind, s.modal.editing)
	}
	if s.pendingLogin != "" {
		t.Errorf("pendingLogin should be cleared; got %q", s.pendingLogin)
	}
	if s.settings.ProviderKeys["umans"] != "sk-test-key-1234" {
		t.Errorf("ProviderKeys[umans] = %q, want sk-test-key-1234", s.settings.ProviderKeys["umans"])
	}
}

// TestLoginKeyEntryEnterUnboundSelect: the app's error messages tell users to
// "run /login first", and /keybinds can clear the select binding — so
// committing a pasted key with Enter must keep working even with select unbound
// (the guaranteed-escape pattern every other select handler already follows).
func TestLoginKeyEntryEnterUnboundSelect(t *testing.T) {
	s := initialSession()
	s.keybinds = defaultKeybinds()
	s.keybinds["select"] = "" // user disabled it via /keybinds
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.ready = true
	s.width, s.height = 80, 24
	s.providerPresets = []providerPreset{{
		ID: "umans", Label: "Umans", Kind: "openai", BaseURL: "https://e/v1",
		EnvVar: "UMANS_API_KEY", HasKey: false, LoggedIn: false,
	}}
	s.openLoginPicker()
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter}) // pick preset (list select has fallback)
	if s.pendingLogin != "umans" || !s.modal.editing {
		t.Fatalf("setup failed: pendingLogin=%q editing=%v", s.pendingLogin, s.modal.editing)
	}
	for _, r := range "sk-x" {
		s.handleModalKey(tea.KeyPressMsg{Code: r, Text: string(r)})
	}
	// Enter to commit — must work even with select unbound.
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.modal.kind != modalNone {
		t.Errorf("modal should close on Enter even with select unbound; kind=%v editing=%v", s.modal.kind, s.modal.editing)
	}
	if s.settings.ProviderKeys["umans"] != "sk-x" {
		t.Errorf("key not committed; ProviderKeys=%v", s.settings.ProviderKeys)
	}
}

func TestLoginKeyEntryAllowsEmptyKeyForLocalProvider(t *testing.T) {
	s := initialSession()
	s.keybinds = defaultKeybinds()
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.ready = true
	s.width, s.height = 80, 24
	s.providerPresets = []providerPreset{{
		ID: "ollama", Label: "Ollama (local)", Kind: "openai",
		BaseURL: "http://localhost:11434/v1", EnvVar: "",
	}}

	s.openLoginPicker()
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.pendingLogin != "ollama" || !s.modal.editing {
		t.Fatalf("setup failed: pendingLogin=%q editing=%v", s.pendingLogin, s.modal.editing)
	}

	wireCoreStub(s)
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter})

	if s.modal.kind != modalNone {
		t.Errorf("modal should close after keyless login; kind=%v", s.modal.kind)
	}
	if _, ok := s.settings.ProviderKeys["ollama"]; ok {
		t.Errorf("keyless login should not persist an Ollama key; ProviderKeys=%v", s.settings.ProviderKeys)
	}
	select {
	case b := <-s.stdinCh:
		var command map[string]any
		if err := json.Unmarshal(b, &command); err != nil {
			t.Fatalf("decode login command: %v", err)
		}
		if command["type"] != "login" || command["preset"] != "ollama" {
			t.Errorf("command = %v, want keyless Ollama login", command)
		}
		if _, ok := command["api_key"]; ok {
			t.Errorf("keyless login unexpectedly included api_key: %v", command)
		}
	default:
		t.Fatal("keyless login did not send a command to the core")
	}
}
