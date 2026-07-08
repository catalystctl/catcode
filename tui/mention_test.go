package main

import (
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
)

// typeRune types a string into the session input via handleKey, the way a
// real terminal would deliver printable runes.
func typeRune(s *session, str string) {
	for _, r := range str {
		s.handleKey(keyMsg(string(r)))
	}
}

// TestMentionActivatesOnAt verifies a bare "@" opens the flyout and lists CWD
// entries, and that Esc closes it.
func TestMentionActivatesOnAt(t *testing.T) {
	s := newMentionSession()
	typeRune(s, "@")
	if !s.mentionActive {
		t.Fatal("typing @ should open the mention flyout")
	}
	if len(s.mentionItems) == 0 {
		t.Fatal("flyout should list CWD entries on bare @")
	}
	// Esc closes the flyout without sending.
	s.handleKey(keyMsg("esc"))
	if s.mentionActive {
		t.Fatal("esc should close the flyout")
	}
}

// TestMentionRecursiveFilter verifies a bare query filters files under the CWD
// and that Tab accepts the selection into the input.
func TestMentionRecursiveFilter(t *testing.T) {
	s := newMentionSession()
	// "main" should match this repo's main.go (the test runs from tui/).
	typeRune(s, "@main")
	if !s.mentionActive {
		t.Fatal("flyout should be active after @main")
	}
	// recursiveSearch fills its cache from a background goroutine (a
	// synchronous walk would freeze the UI on large repos). Poll until the
	// walk completes and main.go appears in the flyout.
	deadline := time.Now().Add(2 * time.Second)
	idx := -1
	for {
		for i, it := range s.mentionItems {
			if it.display == "main.go" {
				idx = i
				break
			}
		}
		if idx >= 0 {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("flyout should contain main.go after walk completes; got %v", itemsDisplay(s.mentionItems))
		}
		s.evalMention() // re-eval against the now-populated cache
		time.Sleep(5 * time.Millisecond)
	}
	// Move the cursor to the main.go entry and accept with Tab.
	s.mentionCursor = idx
	s.handleKey(keyMsg("tab"))
	if s.mentionActive {
		t.Fatal("accepting a file should close the flyout")
	}
	if !strings.Contains(s.input.Value(), "@main.go ") {
		t.Fatalf("input should contain @main.go + trailing space; got %q", s.input.Value())
	}
}

// TestMentionDirCompletionDrillsIn verifies "@../" lists the parent directory
// (files outside the CWD) and that accepting a directory keeps the flyout open
// for further drilling.
func TestMentionDirCompletionDrillsIn(t *testing.T) {
	s := newMentionSession()
	typeRune(s, "@../")
	if !s.mentionActive {
		t.Fatal("flyout should be active after @../")
	}
	if len(s.mentionItems) == 0 {
		t.Fatal("@../ should list the parent directory")
	}
	// The parent (workspace root) must contain a "core" directory.
	idx := -1
	for i, it := range s.mentionItems {
		if it.display == "core/" {
			idx = i
			break
		}
	}
	if idx < 0 {
		t.Fatalf("@../ should list core/ (parent dir); got %v", itemsDisplay(s.mentionItems))
	}
	s.mentionCursor = idx
	s.handleKey(keyMsg("tab"))
	// Accepting a directory keeps the flyout open, drilled into that dir.
	if !s.mentionActive {
		t.Fatal("accepting a directory should keep the flyout open for drilling")
	}
	if !strings.Contains(s.input.Value(), "@../core/") {
		t.Fatalf("input should contain @../core/; got %q", s.input.Value())
	}
}

// TestMentionDoesNotTriggerOnEmail verifies the word-boundary rule: "foo@bar"
// is not treated as a mention.
func TestMentionDoesNotTriggerOnEmail(t *testing.T) {
	s := newMentionSession()
	typeRune(s, "foo@bar")
	if s.mentionActive {
		t.Fatal("foo@bar should not open the mention flyout (no word boundary)")
	}
}

// TestMentionEnterWithNoMatchesSends verifies that when the flyout is open but
// has no matches, Enter falls through and sends the message as typed.
func TestMentionEnterWithNoMatchesSends(t *testing.T) {
	s := newMentionSession()
	typeRune(s, "@zzzznope")
	if !s.mentionActive {
		t.Fatal("flyout should be active")
	}
	if len(s.mentionItems) != 0 {
		t.Fatalf("expected no matches, got %v", itemsDisplay(s.mentionItems))
	}
	// Enter should fall through (no match) and send the line.
	s.handleKey(tea.KeyMsg{Type: tea.KeyEnter})
	if s.mentionActive {
		t.Fatal("sending the message should clear the flyout")
	}
	if s.input.Value() != "" {
		t.Fatalf("input should be cleared after send; got %q", s.input.Value())
	}
}

// newMentionSession builds a minimal idle session ready for input typing.
func newMentionSession() *session {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.models = []modelInfo{{ID: "m1", ContextWindow: 8192}}
	s.modelIdx = 0
	s.authed = true
	s.input.Focus()
	s.layout()
	return s
}

func itemsDisplay(items []mentionItem) []string {
	out := make([]string, len(items))
	for i, it := range items {
		out[i] = it.display
	}
	return out
}

// TestMentionRendersInFullView ensures the flyout renders within the full
// chrome without panicking and reserves layout height.
func TestMentionRendersInFullView(t *testing.T) {
	s := newMentionSession()
	typeRune(s, "@../")
	if !s.mentionActive {
		t.Fatal("flyout should be active")
	}
	s.layout()
	out := s.View()
	if !strings.Contains(out, "navigate") {
		t.Fatal("flyout hint should appear in the full view")
	}
}
