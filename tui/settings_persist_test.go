package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSettingsSavePreservesApproval(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "settings.json")
	// Seed a full settings file as a prior session would leave it.
	initial := map[string]any{
		"approval":         "always",
		"model":            "umans-glm-5.2",
		"active_provider":  "umans",
		"reasoning_effort": "high",
		"theme":            "catalyst",
		"future_key":       "keep-me",
	}
	raw, _ := json.MarshalIndent(initial, "", "  ")
	if err := os.WriteFile(path, raw, 0600); err != nil {
		t.Fatal(err)
	}

	s := &settingsStore{
		path:            path,
		Approval:        "never",
		SelectedModel:   "umans-glm-5.2",
		ActiveProvider:  "umans",
		ReasoningEffort: "high",
		Theme:           "catalyst",
	}
	if err := s.save(); err != nil {
		t.Fatal(err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var onDisk map[string]any
	if err := json.Unmarshal(data, &onDisk); err != nil {
		t.Fatal(err)
	}
	if onDisk["approval"] != "never" {
		t.Fatalf("approval=%v, want never", onDisk["approval"])
	}
	if onDisk["future_key"] != "keep-me" {
		t.Fatalf("unknown key was wiped: %v", onDisk["future_key"])
	}

	// Blank approval in memory must not erase disk.
	s2 := &settingsStore{path: path, Approval: "", SelectedModel: "x"}
	if err := s2.save(); err != nil {
		t.Fatal(err)
	}
	data, _ = os.ReadFile(path)
	_ = json.Unmarshal(data, &onDisk)
	if onDisk["approval"] != "never" {
		t.Fatalf("blank save wiped approval: %v", onDisk["approval"])
	}
}

func TestLoadSettingsApproval(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "settings.json")
	if err := os.WriteFile(path, []byte(`{"approval":"always"}`), 0600); err != nil {
		t.Fatal(err)
	}
	// loadSettings uses settingsPath(); exercise the merge logic via a manual load.
	data, _ := os.ReadFile(path)
	var onDisk settingsStore
	if err := json.Unmarshal(data, &onDisk); err != nil {
		t.Fatal(err)
	}
	if onDisk.Approval != "always" {
		t.Fatalf("got %q", onDisk.Approval)
	}
	if normalizeApproval("") != "destructive" {
		t.Fatal("blank should normalize to destructive")
	}
	if normalizeApproval("ALWAYS") != "always" {
		t.Fatal("case fold")
	}
}

// TestBashTimeoutAndAutoCompactPersist round-trips the two knobs that used to
// be runtime-only (set_config without a settings.json write).
func TestBashTimeoutAndAutoCompactPersist(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "settings.json")

	s := &settingsStore{
		path:            path,
		Approval:        "destructive",
		BashTimeoutSecs: 90,
		AutoCompact:     false,
		IdleTimeout:     180,
	}
	if err := s.save(); err != nil {
		t.Fatal(err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var onDisk map[string]any
	if err := json.Unmarshal(data, &onDisk); err != nil {
		t.Fatal(err)
	}
	if onDisk["bash_timeout_secs"] != float64(90) {
		t.Fatalf("bash_timeout_secs=%v, want 90", onDisk["bash_timeout_secs"])
	}
	if onDisk["auto_compact"] != false {
		t.Fatalf("auto_compact=%v, want false", onDisk["auto_compact"])
	}
	if onDisk["idle_timeout_secs"] != float64(180) {
		t.Fatalf("idle_timeout_secs=%v, want 180 (core-compatible alias)", onDisk["idle_timeout_secs"])
	}

	loaded := loadSettingsFrom(path)
	if loaded.BashTimeoutSecs != 90 {
		t.Fatalf("load BashTimeoutSecs=%d, want 90", loaded.BashTimeoutSecs)
	}
	if loaded.AutoCompact {
		t.Fatal("load AutoCompact=true, want false")
	}
	if loaded.IdleTimeout != 180 {
		t.Fatalf("load IdleTimeout=%d, want 180", loaded.IdleTimeout)
	}
}

// TestLoadSettingsDefaultsFirstBoot: missing file keeps sane first-run defaults
// (especially auto_compact=true, which must not collapse to Go's false zero).
func TestLoadSettingsDefaultsFirstBoot(t *testing.T) {
	path := filepath.Join(t.TempDir(), "missing.json")
	s := loadSettingsFrom(path)
	if s.BashTimeoutSecs != 30 {
		t.Fatalf("BashTimeoutSecs=%d, want 30", s.BashTimeoutSecs)
	}
	if !s.AutoCompact {
		t.Fatal("AutoCompact should default true on first boot")
	}
	if s.IdleTimeout != 120 {
		t.Fatalf("IdleTimeout=%d, want 120", s.IdleTimeout)
	}
	if s.Approval != "destructive" {
		t.Fatalf("Approval=%q, want destructive", s.Approval)
	}
}

// TestLoadSettingsAutoCompactMissingKey: older settings.json without the key
// must keep the true default (not treat absence as false).
func TestLoadSettingsAutoCompactMissingKey(t *testing.T) {
	path := filepath.Join(t.TempDir(), "settings.json")
	if err := os.WriteFile(path, []byte(`{"approval":"always","bash_timeout_secs":45}`), 0600); err != nil {
		t.Fatal(err)
	}
	s := loadSettingsFrom(path)
	if !s.AutoCompact {
		t.Fatal("missing auto_compact key must default to true")
	}
	if s.BashTimeoutSecs != 45 {
		t.Fatalf("BashTimeoutSecs=%d, want 45", s.BashTimeoutSecs)
	}
}

func TestFooterMetricsDefaultsOnAndPersistsOff(t *testing.T) {
	path := filepath.Join(t.TempDir(), "settings.json")
	s := loadSettingsFrom(path)
	if !s.FooterMetrics {
		t.Fatal("footer metrics should default on")
	}
	s.FooterMetrics = false
	if err := s.save(); err != nil {
		t.Fatal(err)
	}
	loaded := loadSettingsFrom(path)
	if loaded.FooterMetrics {
		t.Fatal("footer metrics off should survive reload")
	}
}

func TestFooterMetricsCommandAndRender(t *testing.T) {
	s := initialSession()
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.ready = true
	s.width, s.height = 80, 24
	s.models = []modelInfo{{ID: "glm-5.2", ContextWindow: 128000}}
	s.modelIdx = 0
	s.lastMetrics = json.RawMessage(`{"tps":"51.7","ttft_ms":"180"}`)
	s.settings.FooterMetrics = true
	footer := stripANSI(s.renderFooter())
	for _, want := range []string{"glm-5.2", "52 tok/s", "180ms ttft"} {
		if !strings.Contains(footer, want) {
			t.Fatalf("footer missing %q:\n%s", want, footer)
		}
	}
	_ = s.handleUserLine("/footer-metrics off")
	if s.settings.FooterMetrics || strings.Contains(stripANSI(s.renderFooter()), "tok/s") {
		t.Fatal("footer-metrics off should hide the performance row")
	}
}

// TestToggleCommandsPersistBashAndAutoCompact: slash commands write settings.json.
func TestToggleCommandsPersistBashAndAutoCompact(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")

	s.handleUserLine("/auto-compact off")
	if s.settings.AutoCompact {
		t.Fatal("settings.AutoCompact should be false")
	}
	s.handleUserLine("/bash-timeout 90")
	if s.settings.BashTimeoutSecs != 90 {
		t.Fatalf("settings.BashTimeoutSecs=%d, want 90", s.settings.BashTimeoutSecs)
	}

	loaded := loadSettingsFrom(s.settings.path)
	if loaded.AutoCompact {
		t.Fatal("persisted AutoCompact should be false")
	}
	if loaded.BashTimeoutSecs != 90 {
		t.Fatalf("persisted BashTimeoutSecs=%d, want 90", loaded.BashTimeoutSecs)
	}
}

func TestSettingsSaveClearsKnownValuesAndPreservesUnknown(t *testing.T) {
	path := filepath.Join(t.TempDir(), "settings.json")
	seed := `{"api_key":"secret","model":"old","active_provider":"old","provider_keys":{"old":"secret"},"no_network":true,"mouse_wheel":true,"max_session_tokens":9000,"keybinds":{"send":"x"},"future_key":"keep"}`
	if err := os.WriteFile(path, []byte(seed), 0600); err != nil {
		t.Fatal(err)
	}
	s := &settingsStore{path: path, Approval: "destructive", AutoCompact: true}
	if err := s.save(); err != nil {
		t.Fatal(err)
	}
	var got map[string]any
	b, _ := os.ReadFile(path)
	if err := json.Unmarshal(b, &got); err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{"api_key", "model", "active_provider"} {
		if got[key] != "" {
			t.Fatalf("%s retained stale value %v", key, got[key])
		}
	}
	for _, key := range []string{"no_network", "mouse_wheel"} {
		if got[key] != false {
			t.Fatalf("%s retained stale value %v", key, got[key])
		}
	}
	if got["max_session_tokens"] != float64(0) {
		t.Fatalf("max_session_tokens retained stale value %v", got["max_session_tokens"])
	}
	if len(got["provider_keys"].(map[string]any)) != 0 || len(got["keybinds"].(map[string]any)) != 0 {
		t.Fatal("cleared maps retained stale entries")
	}
	if got["future_key"] != "keep" {
		t.Fatal("unknown setting was not preserved")
	}
}
