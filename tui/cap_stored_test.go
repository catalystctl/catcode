package main

import (
	"strings"
	"testing"
)

func TestCapStoredArgsAndDiff(t *testing.T) {
	small := "hello"
	if got := capStored(small); got != small {
		t.Fatalf("small unchanged: %q", got)
	}
	huge := strings.Repeat("a", maxStoredOutput+100)
	got := capStored(huge)
	if !strings.HasSuffix(got, "\n…[truncated]") {
		t.Fatalf("missing truncation marker: %q", got[len(got)-20:])
	}
	if len(got)-len("\n…[truncated]") != maxStoredOutput {
		t.Fatalf("body len=%d want %d", len(got)-len("\n…[truncated]"), maxStoredOutput)
	}

	s := initialSession()
	s.width = 80
	s.viewport.SetWidth(80)
	b := s.logTool("write_file", huge, false)
	if !strings.HasSuffix(b.args, "\n…[truncated]") {
		t.Fatalf("logTool args not capped")
	}
	s.logApproveDiff("edit", huge, huge)
	var approve *block
	for i := len(s.blocks) - 1; i >= 0; i-- {
		if s.blocks[i].kind == blkApprove {
			approve = s.blocks[i]
			break
		}
	}
	if approve == nil {
		t.Fatal("missing approve block")
	}
	if !strings.HasSuffix(approve.args, "\n…[truncated]") || !strings.HasSuffix(approve.diff, "\n…[truncated]") {
		t.Fatalf("approve args/diff not capped")
	}
}
