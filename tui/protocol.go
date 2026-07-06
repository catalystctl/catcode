package main

import (
	"encoding/json"
	"fmt"
	"strings"
	"time"
)

// ---------------------------------------------------------------------------
// Core protocol types (mirror of the Rust core's stdin/stdout JSON-RPC)
// ---------------------------------------------------------------------------

// modelInfo mirrors a model entry from the core's "ready" event.
type modelInfo struct {
	ID            string `json:"id"`
	Name          string `json:"name"`
	Reasoning     bool   `json:"reasoning"`
	ContextWindow uint32 `json:"context_window"`
	MaxTokens     uint32 `json:"max_tokens"`
	// ThinkingLevels are the reasoning levels the model advertises
	// (e.g. ["low","medium","high"]). Empty when the endpoint provides none;
	// the TUI then falls back to its own low/medium/high set.
	ThinkingLevels []string `json:"thinking_levels"`
	// Vision is true when the model accepts image inputs (from /models/info
	// capabilities.supports_vision; only boolean true counts — "via-handoff"
	// is not native client-side vision). Drives the vision-handoff plugin's routing.
	Vision bool `json:"vision"`
	// Provider is the owning provider name (e.g. "openai", "gemini",
	// "anthropic"), populated by the core's multi-provider aggregation so
	// /models can mix providers and each turn routes to the right endpoint.
	Provider string `json:"provider"`
}

type intercomPrompt struct {
	requestID string
	from      string
	reason    string
	message   string
}

// providerPreset is a built-in first-party provider template (OpenAI/Codex,
// Gemini, Anthropic) advertised by the core via the `provider_presets` event
// (and embedded in `ready`). The /login + /logout pickers use HasKey/Configured/
// LoggedIn to show the right action and prompt for a key when none is set.
type providerPreset struct {
	ID            string   `json:"id"`
	Label         string   `json:"label"`
	Kind          string   `json:"kind"`
	BaseURL       string   `json:"base_url"`
	EnvVar        string   `json:"envVar"`
	AltEnvs       []string `json:"altEnvs"`
	Description   string   `json:"description"`
	HasKey        bool     `json:"hasKey"`
	Configured    bool     `json:"configured"`
	LoggedIn      bool     `json:"loggedIn"`
	SupportsOauth bool     `json:"supportsOauth"`
}

type approvalPrompt struct {
	requestID string
	tool      string
	args      string
	diff      string // unified-diff preview for write/edit/patch (empty for other tools)
}

type subProgressEntry struct {
	runID       string
	agent       string
	toolCount   int
	curTool     string
	toolStart   time.Time
	toolRunning bool
	tokensIn    uint64
	tokensOut   uint64
	started     time.Time
}

// sessionEntry mirrors one element of the core's "sessions" event array.
type sessionEntry struct {
	Name     string `json:"name"`
	Path     string `json:"path"`
	Title    string `json:"title"`
	Messages int    `json:"messages"`
	Mtime    uint64 `json:"mtime"`
	Current  bool   `json:"current"`
}

// memoryEntry mirrors one element of the core's "memory_list" event array
// (id + text + tags; any extra fields the core sends are ignored).
type memoryEntry struct {
	ID   string   `json:"id"`
	Text string   `json:"text"`
	Tags []string `json:"tags"`
}

// skillInfo mirrors one element of the core's "skills" event array. The
// content (SKILL.md body) is sent by the core so /skill:<name> can apply a
// skill without the read_file path restriction blocking global skills.
type skillInfo struct {
	Name        string `json:"name"`
	Description string `json:"description"`
	Location    string `json:"location"`
	Content     string `json:"content"`
}

// coreEvent is one newline-delimited JSON line from the core.
type coreEvent struct {
	Type string          `json:"type"`
	Raw  json.RawMessage `json:"-"`
}

func (e *coreEvent) get(key string) string {
	var m map[string]json.RawMessage
	if err := json.Unmarshal(e.Raw, &m); err != nil {
		return ""
	}
	v, ok := m[key]
	if !ok {
		return ""
	}
	var s string
	if json.Unmarshal(v, &s) == nil {
		return s
	}
	return strings.TrimSpace(string(v))
}

// ---------------------------------------------------------------------------
// Tea messages
// ---------------------------------------------------------------------------

type coreEventMsg struct{ event *coreEvent }
type coreEOFMsg struct{}
type tickMsg struct{ time time.Time }

// ---------------------------------------------------------------------------
// Metrics helpers
// ---------------------------------------------------------------------------

// get reads a trimmed string field from a raw JSON map.
func get(m map[string]json.RawMessage, key string) string {
	v, ok := m[key]
	if !ok {
		return ""
	}
	var s string
	if json.Unmarshal(v, &s) == nil {
		return strings.TrimSpace(s)
	}
	return strings.TrimSpace(string(v))
}

func metricStr(raw json.RawMessage) string {
	if len(raw) == 0 {
		return ""
	}
	var m map[string]json.RawMessage
	if json.Unmarshal(raw, &m) != nil {
		return ""
	}
	tps := get(m, "tps")
	if tps == "" || tps == "null" {
		tps = get(m, "tps_est")
	}
	ttft := get(m, "ttft_ms")
	out := get(m, "tokens_out")
	parts := []string{}
	if tps != "" && tps != "null" {
		parts = append(parts, fmt.Sprintf("T/S %s", tps))
	}
	if ttft != "" && ttft != "null" {
		parts = append(parts, fmt.Sprintf("TTFT %sms", ttft))
	}
	if out != "" && out != "null" {
		parts = append(parts, fmt.Sprintf("out %s", out))
	}
	if len(parts) == 0 {
		return ""
	}
	return strings.Join(parts, " · ")
}
