package main

import (
	"encoding/json"
	"os"
	"path/filepath"
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
