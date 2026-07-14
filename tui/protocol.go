package main

import (
	"encoding/json"
	"fmt"
	"strconv"
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
	requestID  string
	tool       string
	args       string
	diff       string // unified-diff preview for write/edit/patch (empty for other tools)
	expanded   bool   // full diff is visible; toggled by the tool-output keybind
	diffScroll int    // first rendered row of the expanded diff window
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
	Pinned   bool   `json:"pinned"`
}

// memoryEntry mirrors one element of the core's "memory_list" event array
// (id + text + tags; any extra fields the core sends are ignored).
type memoryEntry struct {
	ID   string   `json:"id"`
	Text string   `json:"text"`
	Tags []string `json:"tags"`
}

// contextConsumer is one row of the core's "context_breakdown" event
// top_consumers array (the biggest token consumers in the conversation).
type contextConsumer struct {
	Index   int    `json:"index"`
	Role    string `json:"role"`
	Tokens  uint64 `json:"tokens"`
	Preview string `json:"preview"`
}

// contextBreakdown mirrors the core's "context_breakdown" event payload so the
// TUI can render a /context modal showing where the context budget is spent.
type contextBreakdown struct {
	Total           uint64            `json:"total_tokens"`
	Window          uint64            `json:"context_window"`
	Pct             uint64            `json:"pct"`
	Messages        int               `json:"messages"`
	DigestAt        uint64            `json:"digest_threshold_tokens"`
	CompactAt       uint64            `json:"compact_threshold_tokens"`
	HardLimit       uint64            `json:"hard_limit_tokens"`
	ResponseReserve uint64            `json:"response_reserve_tokens"`
	SafetyMargin    uint64            `json:"safety_margin_tokens"`
	ByRole          map[string]uint64 `json:"by_role"`
	TopConsumers    []contextConsumer `json:"top_consumers"`
}

// usageWindow is one rate-limit / quota row in a provider usage report.
type usageWindow struct {
	ID       string   `json:"id"`
	Label    string   `json:"label"`
	Used     *float64 `json:"used"`
	Limit    *float64 `json:"limit"`
	Unit     string   `json:"unit"`
	ResetsAt *int64   `json:"resets_at"`
	Detail   string   `json:"detail"`
}

// usageReport mirrors the core's "usage" event — plan/window limits for the
// provider that owns the currently selected model (/usage command).
type usageReport struct {
	Provider     string        `json:"provider"`
	ProviderKind string        `json:"provider_kind"`
	Model        string        `json:"model"`
	BaseURL      string        `json:"base_url"`
	Available    bool          `json:"available"`
	Plan         string        `json:"plan"`
	Message      string        `json:"message"`
	Windows      []usageWindow `json:"windows"`
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

// rawKey returns the raw JSON value for a key (e.g. an array/object), so
// callers can unmarshal structured fields themselves without re-parsing Raw.
func (e *coreEvent) rawKey(key string) (json.RawMessage, bool) {
	var m map[string]json.RawMessage
	if err := json.Unmarshal(e.Raw, &m); err != nil {
		return nil, false
	}
	v, ok := m[key]
	return v, ok
}

// ---------------------------------------------------------------------------
// Tea messages
// ---------------------------------------------------------------------------

// Core lifecycle messages carry the subprocess generation that produced them.
// A retry may start a replacement before the old stdout reader has completely
// unwound; generation tags keep stale events/EOF from mutating the new run.
type coreEventMsg struct {
	event *coreEvent
	gen   uint64
}
type coreEOFMsg struct{ gen uint64 }
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

// nullableInt64 parses a coreEvent.get() string into a *int64. Returns nil for
// "" / "null" / unparseable — used for the Umans concurrency fields where nil
// means "absent" (hide the field for used; render ∞ for limit).
func nullableInt64(s string) *int64 {
	if s == "" || s == "null" {
		return nil
	}
	n, err := strconv.ParseInt(s, 10, 64)
	if err != nil {
		return nil
	}
	return &n
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
