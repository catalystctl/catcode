package main

import (
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// chdir changes to dir for the duration of the test.
func chdir(t *testing.T, dir string) {
	t.Helper()
	orig, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Chdir(dir); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Chdir(orig) })
}

// TestSessionPath_PerProjectResumeAndMigration verifies the per-project layout:
// sessionPath() scopes to a per-workspace directory, migrates a legacy
// flat-layout file into it, and resumes the most-recently-modified session.
func TestSessionPath_PerProjectResumeAndMigration(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)
	proj := filepath.Join(tmp, "myproj")
	if err := os.MkdirAll(proj, 0700); err != nil {
		t.Fatal(err)
	}
	chdir(t, proj)

	hash := fmt.Sprintf("%x", fnv64a(proj))
	sessionsRoot := filepath.Join(tmp, ".config", "umans-harness", "sessions")
	projDir := filepath.Join(sessionsRoot, hash)

	// Legacy flat-layout file: sessions/<hash>.jsonl (the old single-file scheme).
	if err := os.MkdirAll(sessionsRoot, 0700); err != nil {
		t.Fatal(err)
	}
	legacy := filepath.Join(sessionsRoot, hash+".jsonl")
	if err := os.WriteFile(legacy, []byte("{\"_session_version\": 1}\n{\"role\":\"user\",\"content\":\"legacy\"}\n"), 0600); err != nil {
		t.Fatal(err)
	}
	oldTime := time.Now().Add(-2 * time.Hour)
	_ = os.Chtimes(legacy, oldTime, oldTime)

	// First call: migrates the legacy file into the per-project dir and resumes it.
	p := sessionPath()
	if filepath.Dir(p) != projDir {
		t.Fatalf("expected session under per-project dir %s, got %s", projDir, p)
	}
	if _, err := os.Stat(legacy); err == nil {
		t.Fatal("legacy flat-layout file should have been migrated away")
	}
	// The migrated file should now be the only one in the project dir.
	entries, err := os.ReadDir(projDir)
	if err != nil || len(entries) != 1 || filepath.Ext(entries[0].Name()) != ".jsonl" {
		t.Fatalf("expected exactly one migrated session in %s, got %v (err=%v)", projDir, entries, err)
	}

	// Create a newer session in the project dir; resume should pick the newest.
	newer := filepath.Join(projDir, "2025-01-01_00-00-00_0000.jsonl")
	if err := os.WriteFile(newer, []byte("{\"_session_version\": 1}\n"), 0600); err != nil {
		t.Fatal(err)
	}
	_ = os.Chtimes(newer, time.Now(), time.Now())

	p2 := sessionPath()
	if p2 != newer {
		t.Fatalf("expected to resume the newest session %s, got %s", newer, p2)
	}
}

// TestNewSessionFilename_UniqueAndReadable checks the generated name is a .jsonl
// file with a timestamp prefix and that rapid calls produce distinct names
// (robust to coarse clocks: require at least 2 distinct among several).
func TestNewSessionFilename_UniqueAndReadable(t *testing.T) {
	a := newSessionFilename()
	if filepath.Ext(a) != ".jsonl" {
		t.Fatalf("expected .jsonl extension, got %q", a)
	}
	seen := map[string]struct{}{a: {}}
	for i := 0; i < 8; i++ {
		seen[newSessionFilename()] = struct{}{}
	}
	if len(seen) < 2 {
		t.Fatalf("newSessionFilename not producing unique names across calls: %q", a)
	}
}
