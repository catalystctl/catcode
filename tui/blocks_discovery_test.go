package main

import (
	"fmt"
	"strings"
	"testing"
	"time"
)

func TestResolvedApprovalHistoryRemovesDecisionControls(t *testing.T) {
	s := initialSession()
	s.width = 80
	s.viewport.SetWidth(80)
	s.logApproveDiff("bash", `{"command":"rm old"}`, "")
	s.resolveLatestApproval("denied")
	b := s.blocks[len(s.blocks)-1]
	got := stripANSI(s.renderBlock(b, 80))
	if !strings.Contains(got, "denied") {
		t.Fatalf("resolved approval missing outcome: %q", got)
	}
	if strings.Contains(got, "approve]") || strings.Contains(got, "] approve") || strings.Contains(got, "] deny") {
		t.Fatalf("resolved approval retained live controls: %q", got)
	}
}

func TestTranscriptTrimAddsVisibleHistoryMarker(t *testing.T) {
	s := initialSession()
	for i := 0; i < maxBlocks+7; i++ {
		b := s.push(blkInfo)
		b.text.WriteString(fmt.Sprintf("event %d", i))
	}
	if len(s.blocks) != maxBlocks {
		t.Fatalf("blocks=%d, want cap %d", len(s.blocks), maxBlocks)
	}
	marker := s.blocks[0]
	if marker.kind != blkTrimmed || marker.trimmed != 8 {
		t.Fatalf("marker=%+v, want 8 hidden blocks", marker)
	}
	if got := stripANSI(s.renderBlock(marker, 80)); !strings.Contains(got, "8 older transcript blocks hidden") {
		t.Fatalf("unexpected marker rendering: %q", got)
	}
}

func TestNearestToolUsesRenderedLineRanges(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 60, 12
	s.viewport.SetWidth(60)
	s.viewport.SetHeight(6)

	first := s.push(blkToolResult)
	first.output = "first\n" + strings.Repeat("short\n", 2)
	first.expanded = true
	spacer := s.push(blkUser)
	spacer.text.WriteString(strings.Repeat("a deliberately tall wrapped message ", 80))
	second := s.push(blkTool)
	second.name, second.output, second.dur = "bash", "second output", time.Second
	s.refresh()
	s.follow = false

	// Put the first card in view. A percentage-based candidate mapping would
	// choose based on transcript proportions rather than the card's actual row.
	s.viewport.SetYOffset(first.renderStart)
	if got := s.nearestToolOutputBlock(); got != first {
		t.Fatalf("nearest at first range=%p, want first=%p (ranges %d-%d, %d-%d)", got, first, first.renderStart, first.renderEnd, second.renderStart, second.renderEnd)
	}
	s.viewport.SetYOffset(max(0, second.renderStart-2))
	if got := s.nearestToolOutputBlock(); got != second {
		t.Fatalf("nearest at second range=%p, want second=%p", got, second)
	}
}
