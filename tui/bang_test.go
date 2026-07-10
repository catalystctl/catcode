package main

import (
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

func TestParseBangCommand(t *testing.T) {
	cases := []struct {
		in      string
		cmd     string
		exclude bool
		ok      bool
	}{
		{"!ls", "ls", false, true},
		{"! git status", "git status", false, true},
		{"!!npm test", "npm test", true, true},
		{"!!  echo hi", "echo hi", true, true},
		{"!", "", false, false},
		{"!!", "", false, false},
		{"! ", "", false, false},
		{"hello", "", false, false},
		{"/help", "", false, false},
		{"!ls -la /tmp", "ls -la /tmp", false, true},
	}
	for _, c := range cases {
		cmd, exclude, ok := parseBangCommand(c.in)
		if ok != c.ok || cmd != c.cmd || exclude != c.exclude {
			t.Errorf("parseBangCommand(%q) = (%q, %v, %v); want (%q, %v, %v)",
				c.in, cmd, exclude, ok, c.cmd, c.exclude, c.ok)
		}
	}
}

func TestBangRunsWhileBusy(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.models = []modelInfo{{ID: "m1"}}
	s.modelIdx = 0
	s.authed = true
	s.busy = true
	s.input.Focus()
	s.layout()

	for _, r := range "!echo hi" {
		s.handleKey(keyMsg(string(r)))
	}
	s.queuedNext = false
	s.handleKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.input.Value() != "" {
		t.Fatalf("input should clear after bang submit; got %q", s.input.Value())
	}
	if s.queuedNext {
		t.Fatal("bang while busy must not queue a follow-up")
	}
}

func TestIsBangCommand(t *testing.T) {
	if !isBangCommand("!ls") {
		t.Fatal("!ls should be a bang command")
	}
	if isBangCommand("!") {
		t.Fatal("bare ! should not be a bang command")
	}
	if isBangCommand("/help") {
		t.Fatal("/help should not be a bang command")
	}
}

func TestBangExecutionFinalizesTool(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.handleCoreEvent(&coreEvent{
		Type: "bash_execution",
		Raw:  []byte(`{"type":"bash_execution","command":"git status","output":"On branch main\n","ok":true,"exclude_from_context":false}`),
	})
	var tool *block
	for _, b := range s.blocks {
		if b.kind == blkTool && b.name == "bash" {
			tool = b
			break
		}
	}
	if tool == nil {
		t.Fatal("expected bash tool block")
	}
	if isInFlight(tool) {
		t.Fatal("bash_execution must finalize the tool (dur > 0); left in-flight forever")
	}
	if !tool.hasOk || !tool.ok {
		t.Fatalf("expected ok result; hasOk=%v ok=%v", tool.hasOk, tool.ok)
	}
	if !strings.Contains(tool.output, "On branch main") {
		t.Fatalf("expected output on tool block; got %q", tool.output)
	}
}
