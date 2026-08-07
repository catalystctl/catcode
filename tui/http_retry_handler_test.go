package main

import (
	"encoding/json"
	"testing"
)

func mkCoreEvent(t *testing.T, line string) *coreEvent {
	t.Helper()
	var ev coreEvent
	raw := append(json.RawMessage(nil), line...)
	var m map[string]json.RawMessage
	if err := json.Unmarshal(raw, &m); err != nil {
		t.Fatalf("unmarshal %s: %v", line, err)
	}
	ev.Raw = raw
	ev.fields = m
	if typRaw, ok := m["type"]; ok {
		var typ string
		_ = json.Unmarshal(typRaw, &typ)
		ev.Type = typ
	}
	return &ev
}

// Mid-stream transport retries re-POST the same turn. The core clears its
// accumulators and sets discard_partial so the TUI drops partial
// assistant/thinking text — otherwise a successful retry would append a
// second copy of the streamed reply.
func TestHttpRetryDiscardPartialDropsLiveAssistant(t *testing.T) {
	s := initialSession()
	s.width = 80

	s.handleCoreEvent(mkCoreEvent(t, `{"type":"delta","text":"partial reply "}`))
	s.handleCoreEvent(mkCoreEvent(t, `{"type":"thinking","text":"scratch"}`))
	if s.cur == nil || s.cur.kind != blkThinking {
		t.Fatalf("expected live thinking block before retry, got %#v", s.cur)
	}
	before := len(s.blocks)
	if before < 2 {
		t.Fatalf("expected assistant+thinking blocks, got %d", before)
	}

	s.handleCoreEvent(mkCoreEvent(t,
		`{"type":"http_retry","attempt":1,"reason":"retryable stream error after partial output","backoff_ms":500,"discard_partial":true}`,
	))

	if s.cur != nil {
		t.Fatalf("expected s.cur cleared after discard_partial, got kind=%v", s.cur.kind)
	}
	for _, b := range s.blocks {
		if b == nil {
			continue
		}
		if b.kind == blkAssistant || b.kind == blkThinking {
			t.Fatalf("partial stream block still present after discard: kind=%v text=%q", b.kind, b.text.String())
		}
	}

	// A successful retry must be able to open a fresh assistant block.
	s.handleCoreEvent(mkCoreEvent(t, `{"type":"delta","text":"full reply"}`))
	if s.cur == nil || s.cur.kind != blkAssistant {
		t.Fatalf("expected fresh assistant after retry, got %#v", s.cur)
	}
	if got := s.cur.text.String(); got != "full reply" {
		t.Fatalf("assistant text = %q, want full reply only (no partial leftover)", got)
	}
}

func TestHttpRetryWithoutDiscardKeepsPartial(t *testing.T) {
	s := initialSession()
	s.width = 80
	s.handleCoreEvent(mkCoreEvent(t, `{"type":"delta","text":"keep me"}`))
	s.handleCoreEvent(mkCoreEvent(t,
		`{"type":"http_retry","attempt":1,"reason":"stream error before first token","backoff_ms":500}`,
	))
	if s.cur == nil || s.cur.kind != blkAssistant {
		t.Fatalf("expected live assistant retained without discard_partial")
	}
	if got := s.cur.text.String(); got != "keep me" {
		t.Fatalf("assistant text = %q, want kept partial", got)
	}
}
