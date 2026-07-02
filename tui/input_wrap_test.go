package main

import (
	"strings"
	"testing"
)

func newInputSession(t *testing.T, width int) *session {
	t.Helper()
	s := initialSession()
	s.ready = true
	s.width, s.height = width, 40
	s.authed = true
	s.input.Focus()
	s.layout()
	return s
}

// TestInputBoxEmptyIsThreeLines: an empty input renders the single-line
// placeholder box (top border + placeholder + bottom border).
func TestInputBoxEmptyIsThreeLines(t *testing.T) {
	s := newInputSession(t, 80)
	box := stripANSI(s.renderInputBox())
	lines := strings.Split(box, "\n")
	if len(lines) != 3 {
		t.Fatalf("empty input box should be 3 lines, got %d:\n%s", len(lines), box)
	}
	if !strings.HasPrefix(lines[0], "╭") || !strings.HasPrefix(lines[2], "╰") {
		t.Fatalf("input box missing rounded borders:\n%s", box)
	}
	if !strings.Contains(lines[1], "Chat with the agent") {
		t.Fatalf("input box missing placeholder:\n%s", box)
	}
}

// TestInputBoxWrapsLongMessage: a value longer than the box width soft-wraps
// onto multiple rows instead of scrolling one line — every char is still
// visible and each content row is boxed with side borders.
func TestInputBoxWrapsLongMessage(t *testing.T) {
	s := newInputSession(t, 40) // inner width = 36
	s.input.SetValue(strings.Repeat("a", 80))
	s.layout() // recompute now that the input grew

	box := stripANSI(s.renderInputBox())
	lines := strings.Split(box, "\n")
	if len(lines) <= 3 {
		t.Fatalf("a long message should wrap to multiple rows (box > 3 lines), got %d:\n%s", len(lines), box)
	}
	// borders bookend the box; every interior row must be side-bordered.
	if !strings.HasPrefix(lines[0], "╭") || !strings.HasPrefix(lines[len(lines)-1], "╰") {
		t.Fatalf("box missing top/bottom border:\n%s", box)
	}
	for i := 1; i < len(lines)-1; i++ {
		if !strings.HasPrefix(lines[i], "│") || !strings.HasSuffix(lines[i], "│") {
			t.Fatalf("content row %d missing side borders: %q", i, lines[i])
		}
	}
	// wrapping must not drop any characters
	if got := strings.Count(box, "a"); got != 80 {
		t.Fatalf("wrapping dropped/duplicated chars: want 80 a's, got %d", got)
	}
	// the box grew to fit the wrapped content
	if h := s.inputBoxHeight(); h <= 3 {
		t.Fatalf("inputBoxHeight should grow past 3 for a wrapped message, got %d", h)
	}
}

// TestInputBoxCapsVeryLong: a pathologically long message is windowed so the
// box never eats the whole screen (bounded height).
func TestInputBoxCapsVeryLong(t *testing.T) {
	s := newInputSession(t, 40) // inner width = 36
	s.input.SetValue(strings.Repeat("b", 1000))
	s.layout()

	box := s.renderInputBox()
	h := lipglossHeight(box)
	// maxInputLines content + up to 2 "…" markers + 2 borders.
	const ceiling = maxInputLines + 2 + 2
	if h > ceiling {
		t.Fatalf("box height %d exceeds cap %d for a 1000-char message:\n%s", h, ceiling, box)
	}
}

// TestInputBoxShrinksAfterClear: after wrapping, clearing the input returns
// the box to the single-line placeholder.
func TestInputBoxShrinksAfterClear(t *testing.T) {
	s := newInputSession(t, 40)
	s.input.SetValue(strings.Repeat("c", 80))
	s.layout()
	if s.inputBoxHeight() <= 3 {
		t.Fatalf("expected wrapped box to be > 3 lines")
	}
	s.input.Reset()
	s.layout()
	if h := s.inputBoxHeight(); h != 3 {
		t.Fatalf("after clearing, box should be 3 lines, got %d", h)
	}
}

// lipglossHeight is a thin alias to avoid importing lipgloss in the test.
func lipglossHeight(s string) int {
	// count newlines + 1 of the ANSI-stripped string
	clean := stripANSI(s)
	if clean == "" {
		return 0
	}
	return strings.Count(clean, "\n") + 1
}
