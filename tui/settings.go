package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// ---------------------------------------------------------------------------
// Settings persistence (TUI-owned prefs)
//
// Stored as JSON at ~/.config/umans-harness/settings.json with 0600 perms.
// Atomic write via temp-file + rename so a crash never corrupts the file.
// The API key is stored locally (same trust model as ~/.pi/agent/auth.json);
// the file is mode 0600 and the key is masked in the UI.
// ---------------------------------------------------------------------------

type settingsStore struct {
	path            string
	APIKey          string `json:"api_key,omitempty"`
	SelectedModel   string `json:"model,omitempty"`
	Approval        string `json:"approval,omitempty"`
	ReasoningEffort string `json:"reasoning_effort,omitempty"`
	Theme           string `json:"theme,omitempty"`
	ThinkExpanded   bool   `json:"think_expanded,omitempty"`
	// Production knobs (item 3/7): passed to the core on launch.
	Sandbox          string `json:"sandbox,omitempty"` // none | firejail
	NoNetwork        bool   `json:"no_network,omitempty"`
	IdleTimeout      int    `json:"idle_timeout,omitempty"`       // seconds
	MaxSessionTokens int    `json:"max_session_tokens,omitempty"` // 0=unlimited
	MouseWheel       bool   `json:"mouse_wheel,omitempty"`        // opt-in wheel scroll (off keeps native click-drag copy)

	// Custom providers (openai/anthropic endpoints). ActiveProvider is the
	// provider name selected in the TUI; the core re-applies it on launch via
	// the `set_provider` command. ProviderKeys holds a per-provider API key
	// (keyed by provider name) so each endpoint keeps its own secret; it
	// supersedes the legacy single APIKey when set. Stored in this 0600 file.
	ActiveProvider string            `json:"active_provider,omitempty"`
	ProviderKeys   map[string]string `json:"provider_keys,omitempty"`

	// Custom keybindings (TUI-only). Maps action name → canonical key (the
	// string tea.KeyMsg.String() produces). Only user overrides are stored; the
	// full effective map (defaults + overrides) is computed at startup via
	// effectiveKeybinds(). See keybinds.go.
	Keybinds map[string]string `json:"keybinds,omitempty"`
}

func configDir() string {
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".config", "umans-harness")
}

// sessionPath returns the JSONL conversation file the core resumes from.
// Sessions are scoped per workspace: each project gets its own directory
// (~/.config/umans-harness/sessions/<hex(cwd)>) holding an unlimited number of
// session files. On launch we resume the most recently modified one (crash
// recovery / continuity); if none exists we start a fresh timestamped file.
// A legacy flat-layout file (sessions/<hex>.jsonl) is migrated into the dir
// once so existing histories aren't lost.
func sessionPath() string {
	dir := sessionsDir()
	_ = os.MkdirAll(dir, 0700)
	migrateLegacySession(dir)
	entries, err := os.ReadDir(dir)
	if err == nil {
		var best string
		var bestMtime time.Time
		for _, e := range entries {
			if e.IsDir() || !strings.HasSuffix(e.Name(), ".jsonl") {
				continue
			}
			if fi, err := e.Info(); err == nil {
				mt := fi.ModTime()
				if best == "" || mt.After(bestMtime) {
					best, bestMtime = e.Name(), mt
				}
			}
		}
		if best != "" {
			return filepath.Join(dir, best)
		}
	}
	return filepath.Join(dir, newSessionFilename())
}

// sessionsDir is the per-workspace directory holding that project's session
// files. Distinct projects (distinct cwd) get distinct directories.
func sessionsDir() string {
	cwd, err := os.Getwd()
	if err != nil {
		cwd = "."
	}
	if abs, err := filepath.Abs(cwd); err == nil {
		cwd = abs
	}
	return filepath.Join(configDir(), "sessions", fmt.Sprintf("%x", fnv64a(cwd)))
}

// newSessionFilename returns a unique, human-readable timestamped name for a
// new session file (date_time + nanosecond suffix for uniqueness).
func newSessionFilename() string {
	t := time.Now()
	return fmt.Sprintf("%s_%09d.jsonl", t.Format("2006-01-02_15-04-05"), t.Nanosecond())
}

// migrateLegacySession moves a legacy flat-layout file (sessions/<hex>.jsonl)
// into the per-project directory, named by its original mtime. Runs once:
// after the move the legacy path no longer exists.
func migrateLegacySession(dir string) {
	legacy := filepath.Join(filepath.Dir(dir), filepath.Base(dir)+".jsonl")
	info, err := os.Stat(legacy)
	if err != nil || info.IsDir() {
		return
	}
	name := info.ModTime().Format("2006-01-02_15-04-05") + ".jsonl"
	dst := filepath.Join(dir, name)
	if _, err := os.Stat(dst); err == nil {
		dst = filepath.Join(dir, newSessionFilename())
	}
	_ = os.Rename(legacy, dst)
}

// settingsPath is the TUI prefs file (api key, model, theme, ...).
func settingsPath() string {
	return filepath.Join(configDir(), "settings.json")
}

// fnv64a: 64-bit FNV-1a, same algorithm the core uses for line hashes.
func fnv64a(s string) uint64 {
	const offset uint64 = 0xcbf29ce484222325
	const prime uint64 = 0x100000001b3
	h := offset
	for i := 0; i < len(s); i++ {
		h ^= uint64(s[i])
		h *= prime
	}
	return h
}

// loadSettings reads the persisted prefs, returning a zero-value store on any
// error (missing file is not an error — first run).
func loadSettings() *settingsStore {
	s := &settingsStore{path: settingsPath(), ReasoningEffort: "high", Approval: "destructive"}
	data, err := os.ReadFile(s.path)
	if err != nil {
		return s
	}
	// Keep defaults for fields absent from the file.
	var onDisk settingsStore
	if err := json.Unmarshal(data, &onDisk); err != nil {
		return s
	}
	if onDisk.APIKey != "" {
		s.APIKey = onDisk.APIKey
	}
	if onDisk.SelectedModel != "" {
		s.SelectedModel = onDisk.SelectedModel
	}
	if onDisk.Approval != "" {
		s.Approval = onDisk.Approval
	}
	if onDisk.ReasoningEffort != "" {
		s.ReasoningEffort = onDisk.ReasoningEffort
	}
	if onDisk.Theme != "" {
		s.Theme = onDisk.Theme
	}
	s.ThinkExpanded = onDisk.ThinkExpanded
	if onDisk.Sandbox != "" {
		s.Sandbox = onDisk.Sandbox
	} else {
		s.Sandbox = "none"
	}
	s.NoNetwork = onDisk.NoNetwork
	if onDisk.IdleTimeout > 0 {
		s.IdleTimeout = onDisk.IdleTimeout
	} else {
		s.IdleTimeout = 120
	}
	s.MaxSessionTokens = onDisk.MaxSessionTokens
	s.MouseWheel = onDisk.MouseWheel
	if onDisk.ActiveProvider != "" {
		s.ActiveProvider = onDisk.ActiveProvider
	}
	s.ProviderKeys = map[string]string{}
	for k, v := range onDisk.ProviderKeys {
		if v != "" {
			s.ProviderKeys[k] = v
		}
	}
	// Custom keybinds: store only the user overrides (non-default bindings).
	// Empty string is a valid override meaning "disabled" (kb returns false),
	// so we keep it rather than falling back to the default.
	// The full effective map is computed in initialSession via effectiveKeybinds().
	if onDisk.Keybinds != nil {
		s.Keybinds = map[string]string{}
		for k, v := range onDisk.Keybinds {
			s.Keybinds[k] = v // keep empty (disabled) values
		}
	}
	return s
}

// save writes the store atomically with 0600 perms.
func (s *settingsStore) save() error {
	dir := filepath.Dir(s.path)
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	data, err := json.MarshalIndent(s, "", "  ")
	if err != nil {
		return err
	}
	tmp := s.path + ".tmp"
	if err := os.WriteFile(tmp, data, 0600); err != nil {
		return err
	}
	return os.Rename(tmp, s.path)
}
