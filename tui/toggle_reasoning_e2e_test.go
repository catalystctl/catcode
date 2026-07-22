package main

import (
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// resetThinkExpanded isolates each test from the persisted ThinkExpanded
// setting. The real toggle handler calls s.settings.save(), which would write
// ThinkExpanded to the user's REAL config file (~/.config/catalyst-code) — so we
// redirect save() to a throwaway temp path AND reset the in-memory flag.
func resetThinkExpanded(t *testing.T, s *session) {
	s.thinkExpanded = false
	s.settings.ThinkExpanded = false
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
}

// TestToggleReasoningKeypressEndToEnd drives the ACTUAL ctrl+t keypress through
// handleKey (the real dispatcher) and checks the viewport content the user sees
// — not just renderBlocks — for a finalized reasoning block.
func TestToggleReasoningKeypressEndToEnd(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width = 90
	s.height = 30
	s.authed = true
	resetThinkExpanded(t, s)
	s.layout()

	tb := s.push(blkThinking)
	tb.text.WriteString("alpha observation\nbeta deduction\ngamma conclusion")
	s.cur = nil
	s.invalidateAll()
	s.refresh()

	if !strings.Contains(stripANSI(s.renderBlocks()), "reasoning") {
		t.Fatalf("setup: collapsed pill missing")
	}

	s.handleKey(keyMsg("ctrl+t"))

	seen := stripANSI(s.viewport.View())
	if !strings.Contains(seen, "beta deduction") {
		t.Errorf("finalized reasoning did not expand via ctrl+t:\n%s", seen)
	}
}

// TestToggleReasoningWhileStreaming covers the live case: the reasoning block
// is s.cur (still streaming) when the user hits ctrl+t.
func TestToggleReasoningWhileStreaming(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width = 90
	s.height = 30
	resetThinkExpanded(t, s)
	s.layout()

	tb := s.push(blkThinking) // becomes s.cur
	tb.text.WriteString("streaming thought one\nstreaming thought two")
	s.invalidateAll()
	s.refresh()

	s.handleKey(keyMsg("ctrl+t"))

	seen := stripANSI(s.viewport.View())
	if !strings.Contains(seen, "streaming thought two") {
		t.Errorf("live reasoning did not expand via ctrl+t:\n%s", seen)
	}
}

// TestToggleReasoningLargeExpand simulates the user's real scenario: a large
// (~hundreds of lines) finalized reasoning block expanded via ctrl+t. It
// verifies the expand actually completes (Glamour cold-render doesn't error or
// return empty) and measures the cold-render cost.
func TestToggleReasoningLargeExpand(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width = 90
	s.height = 30
	resetThinkExpanded(t, s)
	s.layout()

	// ~300 lines of prose reasoning (Glamour cold cache).
	var body strings.Builder
	for i := 0; i < 300; i++ {
		body.WriteString("Considering the approach for step ")
		body.WriteString(strings.Repeat("x", i%7))
		body.WriteString(" and its tradeoffs in this context.\n")
	}
	tb := s.push(blkThinking)
	tb.text.WriteString(body.String())
	s.cur = nil
	s.invalidateAll()
	s.refresh()

	if !strings.Contains(stripANSI(s.renderBlocks()), "reasoning") {
		t.Fatalf("setup: collapsed pill missing")
	}

	start := time.Now()
	s.handleKey(keyMsg("ctrl+t"))
	elapsed := time.Since(start)

	seen := stripANSI(s.viewport.View())
	if !strings.Contains(seen, "approach for step") {
		t.Errorf("large reasoning did not expand via ctrl+t (elapsed %v):\n%s", elapsed, seen[:min(len(seen), 400)])
	}
	t.Logf("large expand cold-render elapsed: %v", elapsed)
}
