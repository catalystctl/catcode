package main

import (
	"strings"
	"testing"
)

// TestToggleReasoningExpands reproduces the user action: a finalized reasoning
// block (collapsed by default) is expanded via the ctrl+t toggle and must
// actually render its full markdown content, not stay on the collapsed pill.
func TestToggleReasoningExpands(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width = 90
	s.height = 30
	resetThinkExpanded(t, s)
	s.layout()

	// Finalized reasoning block, collapsed by default (thinkExpanded=false).
	const reasoning = "Let me think about this carefully.\nFirst, consider the data flow.\nThen the edge cases.\nFinally the API surface."
	tb := s.push(blkThinking)
	tb.text.WriteString(reasoning)
	s.cur = nil // finalize
	s.invalidateAll()
	s.refresh()

	collapsed := stripANSI(s.renderBlocks())
	t.Logf("COLLAPSED:\n%s", collapsed)
	if !strings.Contains(collapsed, "reasoning") || !strings.Contains(collapsed, "expand") {
		t.Fatalf("collapsed render should show the reasoning pill, got:\n%s", collapsed)
	}
	if strings.Contains(collapsed, "data flow") {
		t.Fatalf("collapsed render leaked full reasoning body:\n%s", collapsed)
	}

	// Simulate ctrl+t: replicate toggle_reasoning handler exactly.
	s.thinkExpanded = !s.thinkExpanded
	s.settings.ThinkExpanded = s.thinkExpanded
	for _, b := range s.blocks {
		if b.kind == blkThinking {
			b.collapsed = !s.thinkExpanded
			b.renderStr = ""
			b.renderLen = 0
		}
	}
	s.invalidateAll()
	s.refresh()

	expanded := stripANSI(s.renderBlocks())
	t.Logf("EXPANDED:\n%s", expanded)
	for _, want := range []string{"data flow", "edge cases", "API surface"} {
		if !strings.Contains(expanded, want) {
			t.Errorf("expanded render missing %q (toggle did not expand):\n%s", want, expanded)
		}
	}
	if strings.Contains(expanded, "ctrl+t expand") {
		t.Errorf("expanded render still shows the collapsed pill hint:\n%s", expanded)
	}
}

// TestToggleReasoningCollapseRoundTrip ensures expand→collapse still collapses
// (the cache must not pin the expanded render after a second toggle).
func TestToggleReasoningCollapseRoundTrip(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width = 90
	s.height = 30
	resetThinkExpanded(t, s)
	s.layout()

	tb := s.push(blkThinking)
	tb.text.WriteString("step one\nstep two\nstep three")
	s.cur = nil
	s.invalidateAll()
	s.refresh()

	// expand
	s.thinkExpanded = true
	for _, b := range s.blocks {
		if b.kind == blkThinking {
			b.collapsed = false
			b.renderStr = ""
			b.renderLen = 0
		}
	}
	s.invalidateAll()
	s.refresh()
	if exp := stripANSI(s.renderBlocks()); !strings.Contains(exp, "step two") {
		t.Fatalf("expand step missing body:\n%s", exp)
	}

	// collapse
	s.thinkExpanded = false
	for _, b := range s.blocks {
		if b.kind == blkThinking {
			b.collapsed = true
			b.renderStr = ""
			b.renderLen = 0
		}
	}
	s.invalidateAll()
	s.refresh()
	col := stripANSI(s.renderBlocks())
	if strings.Contains(col, "step two") {
		t.Fatalf("collapse step leaked body (cache pinned expanded render):\n%s", col)
	}
	if !strings.Contains(col, "reasoning") {
		t.Fatalf("collapse step lost the pill:\n%s", col)
	}
}
