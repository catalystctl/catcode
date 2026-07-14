package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

func TestCoreBinaryPathHonorsExplicitOverride(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "intentionally-missing-core")
	t.Setenv("CATCODE_CORE", missing)
	got := coreBinaryPath()
	want, _ := filepath.Abs(missing)
	if got != want {
		t.Fatalf("coreBinaryPath=%q, want explicit %q", got, want)
	}
	if _, err := os.Stat(got); !os.IsNotExist(err) {
		t.Fatalf("test override unexpectedly exists: %v", err)
	}
}

func rawEvent(t *testing.T, typ string, fields map[string]any) *coreEvent {
	t.Helper()
	fields["type"] = typ
	raw, err := json.Marshal(fields)
	if err != nil {
		t.Fatal(err)
	}
	return &coreEvent{Type: typ, Raw: raw}
}

func TestProviderChangedUsesDecodedStrings(t *testing.T) {
	s := initialSession()
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.handleCoreEvent(rawEvent(t, "provider_changed", map[string]any{
		"provider": "openai", "kind": "openai", "has_key": false,
	}))
	if s.activeProvider != "openai" || s.providerKind != "openai" {
		t.Fatalf("provider=%q kind=%q", s.activeProvider, s.providerKind)
	}
	if s.settings.ActiveProvider != "openai" {
		t.Fatalf("selection not persisted: %q", s.settings.ActiveProvider)
	}
}

func TestLegacyKeyCannotCrossProviderBoundary(t *testing.T) {
	s := initialSession()
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.settings.APIKey = "legacy-secret"
	s.settings.ActiveProvider = ""
	s.settings.ProviderKeys = nil
	s.activeProvider = "umans"
	s.migrateLegacyProviderKey("umans")
	if got := s.providerKey("umans"); got != "legacy-secret" {
		t.Fatalf("migrated key=%q", got)
	}
	s.activeProvider = "openai"
	s.settings.ActiveProvider = "openai"
	if got := s.providerKey("openai"); got != "" {
		t.Fatalf("legacy credential crossed providers: %q", got)
	}
}

func TestAccumulateSavedUsesPositiveReclaimedDelta(t *testing.T) {
	s := initialSession()
	s.accumulateSaved(rawEvent(t, "compacted", map[string]any{"before_tokens": 100, "after_tokens": 40}))
	if s.tokensSaved != 60 {
		t.Fatalf("tokensSaved=%d, want 60", s.tokensSaved)
	}
	s.accumulateSaved(rawEvent(t, "compacted", map[string]any{"before_tokens": 40, "after_tokens": 100}))
	if s.tokensSaved != 60 {
		t.Fatalf("invalid growth event changed tokensSaved=%d", s.tokensSaved)
	}
}

func TestCoreFailureIsPersistentAndOffersRecovery(t *testing.T) {
	s := initialSession()
	s.viewport.SetWidth(80)
	s.viewport.SetHeight(20)
	s.coreLifecycle = coreFailed
	s.coreFailure = "core exploded"
	view := s.renderBlocks()
	if !strings.Contains(view, "core exploded") || !strings.Contains(view, "retry") || !strings.Contains(view, "debug.jsonl") {
		t.Fatalf("failure screen lacks recovery detail: %q", view)
	}
}

func TestReadyEventReplacesCachedStartingWelcome(t *testing.T) {
	s := initialSession()
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.ready = true
	s.width, s.height = 80, 24
	s.coreStartGen = 1
	s.coreLifecycle = coreStarting
	s.layout()

	if view := stripANSI(s.viewport.View()); !strings.Contains(view, "Starting") {
		t.Fatalf("precondition: viewport does not show startup state:\n%s", view)
	}

	s.handleCoreEvent(rawEvent(t, "ready", map[string]any{
		"models": []modelInfo{{
			ID:            "umans-glm-5.2",
			Name:          "GLM 5.2",
			Provider:      "umans",
			ContextWindow: 131072,
			MaxTokens:     8192,
		}},
		"authed":            true,
		"provider":          "umans",
		"providerKind":      "openai",
		"providers":         []string{"umans"},
		"providerPresets":   []providerPreset{},
		"approval":          "destructive",
		"bash_timeout_secs": 30,
		"auto_compact":      true,
	}))

	view := stripANSI(s.viewport.View())
	if strings.Contains(view, "Starting") || strings.Contains(view, "Get started") {
		t.Fatalf("ready event left a stale startup screen:\n%s", view)
	}
	if !strings.Contains(view, "Understand this repository") {
		t.Fatalf("authenticated welcome examples were not rendered:\n%s", view)
	}
	if !s.authed || s.coreLifecycle != coreReady {
		t.Fatalf("ready state not applied: authed=%v lifecycle=%v", s.authed, s.coreLifecycle)
	}
}

func TestWelcomeArrowsDoNotStealDraftNavigation(t *testing.T) {
	s := initialSession()
	s.authed = true
	s.input.SetValue("first\nsecond")
	original := s.welcomeIdx
	s.handleKey(tea.KeyPressMsg{Code: tea.KeyDown})
	if s.welcomeIdx != original {
		t.Fatal("welcome cursor moved while composer had a draft")
	}
}
