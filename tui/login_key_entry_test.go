package main

import (
	"path/filepath"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
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
	s.handleModalKey(tea.KeyMsg{Type: tea.KeyEnter})
	if s.modal.editing != true {
		t.Fatalf("expected editing=true after picking preset, got false (pendingLogin=%q)", s.pendingLogin)
	}
	if s.pendingLogin != "umans" {
		t.Fatalf("expected pendingLogin=umans, got %q", s.pendingLogin)
	}

	for _, r := range "sk-test-key-1234" {
		s.handleModalKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{r}})
	}
	if got := s.modal.editBuf.Value(); got != "sk-test-key-1234" {
		t.Fatalf("editBuf value after typing = %q, want sk-test-key-1234", got)
	}

	// Press Enter to commit.
	s.handleModalKey(tea.KeyMsg{Type: tea.KeyEnter})

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
// "run /key sk-... first", and /keybinds can clear the select binding — so
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
	s.handleModalKey(tea.KeyMsg{Type: tea.KeyEnter}) // pick preset (list select has fallback)
	if s.pendingLogin != "umans" || !s.modal.editing {
		t.Fatalf("setup failed: pendingLogin=%q editing=%v", s.pendingLogin, s.modal.editing)
	}
	for _, r := range "sk-x" {
		s.handleModalKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{r}})
	}
	// Enter to commit — must work even with select unbound.
	s.handleModalKey(tea.KeyMsg{Type: tea.KeyEnter})
	if s.modal.kind != modalNone {
		t.Errorf("modal should close on Enter even with select unbound; kind=%v editing=%v", s.modal.kind, s.modal.editing)
	}
	if s.settings.ProviderKeys["umans"] != "sk-x" {
		t.Errorf("key not committed; ProviderKeys=%v", s.settings.ProviderKeys)
	}
}

// TestKeyCommandSetsActiveProviderKey: /key <value> (deleted then restored) must
// set the API key for the active provider, since the app's "not authenticated"
// errors direct users to "run /key sk-... first".
func TestKeyCommandSetsActiveProviderKey(t *testing.T) {
	s := initialSession()
	s.keybinds = defaultKeybinds()
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.ready = true
	s.width, s.height = 80, 24
	s.activeProvider = "umans"

	// Simulate the user typing "/key sk-abc" and pressing Enter.
	cmd := s.handleUserLine("/key sk-abc")
	_ = cmd

	if s.settings.ProviderKeys["umans"] != "sk-abc" {
		t.Errorf("ProviderKeys[umans] = %q, want sk-abc", s.settings.ProviderKeys["umans"])
	}
	if s.settings.APIKey != "sk-abc" {
		t.Errorf("APIKey = %q, want sk-abc", s.settings.APIKey)
	}
	// sendCore is a no-op without a running core; assert it didn't crash and
	// state is consistent. Re-auth path should now report a key is present.
	if !s.sendProviderKey("umans") {
		t.Error("sendProviderKey(umans) should report a key is available after /key")
	}
}
