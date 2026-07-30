package main

import (
	"encoding/json"
	"testing"
)

// TestDeltaEventAppendsAssistantText guards against a regression where the
// `case "delta":` label was accidentally deleted (commit 18e41ab, when the
// provider_models_preview case was added above it). Without that label the
// assistant-text handler body was swallowed into provider_models_preview,
// which returns early unless the custom-provider modal is open — so every
// streaming assistant token (core's Event::new("delta").with("text",…)) was
// silently dropped and the assistant's prose replies never rendered.
func TestDeltaEventAppendsAssistantText(t *testing.T) {
	s := initialSession()
	s.width = 80

	// Build events the same way the core reader does (main.go): parse the
	// line into fields and set ev.Type so handleCoreEvent's switch matches.
	mk := func(line string) *coreEvent {
		var ev coreEvent
		raw := append(json.RawMessage(nil), line...)
		var m map[string]json.RawMessage
		if err := json.Unmarshal(raw, &m); err != nil {
			t.Fatalf("unmarshal %s: %v", line, err)
		}
		ev.Raw = raw
		ev.fields = m
		if t, ok := m["type"]; ok {
			var typ string
			_ = json.Unmarshal(t, &typ)
			ev.Type = typ
		}
		return &ev
	}

	// First delta opens a live assistant block; a second continues it.
	s.handleCoreEvent(mk(`{"type":"delta","text":"hello "}`))
	s.handleCoreEvent(mk(`{"type":"delta","text":"world"}`))

	if s.cur == nil || s.cur.kind != blkAssistant {
		t.Fatalf("expected a live blkAssistant after delta events, got cur=%v", s.cur)
	}
	if got := s.cur.text.String(); got != "hello world" {
		t.Fatalf("assistant text = %q, want %q (delta events not appended)", got, "hello world")
	}
	// The block must be in the transcript, not just the live cursor.
	var found bool
	for _, b := range s.blocks {
		if b != nil && b.kind == blkAssistant {
			found = true
			break
		}
	}
	if !found {
		t.Fatal("no blkAssistant in s.blocks after delta events")
	}
}
