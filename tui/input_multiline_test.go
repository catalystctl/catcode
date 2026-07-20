package main

import (
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

// TestInputBoxRendersLiteralNewline: a value containing a line break renders
// each line on its own boxed row instead of folding the '\n' into a wrapped
// line (which previously broke the box width math and showed a stray glyph).
func TestInputBoxRendersLiteralNewline(t *testing.T) {
	s := newInputSession(t, 40) // inner width = 36
	s.input.SetValue("line1\nline2")
	s.layout()

	box := stripANSI(s.renderInputBox())
	lines := strings.Split(box, "\n")
	// top border + 2 content rows + bottom border
	if len(lines) != 4 {
		t.Fatalf("two-line input should render 4 composer rows (got %d):\n%s", len(lines), box)
	}
	// both source lines must appear on their own bordered rows, neither dropped
	if !strings.Contains(lines[1], "line1") {
		t.Fatalf("first line missing from row 1: %q\n%s", lines[1], box)
	}
	if !strings.Contains(lines[2], "line2") {
		t.Fatalf("second line missing from row 2: %q\n%s", lines[2], box)
	}
	if !strings.Contains(lines[1], "❯ ") || !strings.HasPrefix(lines[2], "│   ") {
		t.Fatalf("composer rows are not prompt-aligned: %q / %q", lines[1], lines[2])
	}
	if h := s.inputBoxHeight(); h != 4 {
		t.Fatalf("inputBoxHeight should be 4 for a two-line message, got %d", h)
	}
}

// TestInputBoxNewlineNotDropped: the newline character must not be silently
// dropped or rendered as a visible control glyph — the two lines stay intact.
func TestInputBoxNewlineNotDropped(t *testing.T) {
	s := newInputSession(t, 60)
	s.input.SetValue("a\nb")
	s.layout()
	box := stripANSI(s.renderInputBox())
	// exactly one '\n' is structural (between the two lines) and must not be
	// rendered as a literal glyph on a content row.
	for _, ln := range strings.Split(box, "\n")[1:] {
		if strings.ContainsAny(ln, "\n") {
			t.Fatalf("content row contains a literal newline: %q", ln)
		}
	}
	if c := strings.Count(box, "a"); c != 1 {
		t.Fatalf("expected exactly 1 'a', got %d", c)
	}
	if c := strings.Count(box, "b"); c != 1 {
		t.Fatalf("expected exactly 1 'b', got %d", c)
	}
}

// TestInputBoxTrailingNewlineShowsBlankRow: a trailing line break keeps a
// final empty row so the user sees the blank line they entered.
func TestInputBoxTrailingNewlineShowsBlankRow(t *testing.T) {
	s := newInputSession(t, 40)
	s.input.SetValue("hello\n")
	setInputCursor(&s.input, len([]rune(s.input.Value()))) // cursor at very end
	s.layout()
	box := stripANSI(s.renderInputBox())
	lines := strings.Split(box, "\n")
	// top border + "hello" + blank + bottom border = 4 rows
	if len(lines) != 4 {
		t.Fatalf("trailing newline should render a blank row (got %d rows):\n%s", len(lines), box)
	}
	// row 3 is a blank continuation aligned under the prompt.
	content := strings.TrimSuffix(strings.TrimPrefix(lines[2], "│"), "│")
	if strings.TrimSpace(content) != "" {
		t.Fatalf("row 3 should be a blank boxed line, got %q", lines[2])
	}
}

// TestInsertNewlineAtCursor inserts a line break at the cursor and leaves the
// cursor on the new (second) line.
func TestInsertNewlineAtCursor(t *testing.T) {
	s := newInputSession(t, 60)
	s.input.SetValue("hello world")
	setInputCursor(&s.input, 5) // between "hello" and " world"
	s.insertNewline()

	if got := s.input.Value(); got != "hello\n world" {
		t.Fatalf("value after insertNewline = %q, want %q", got, "hello\n world")
	}
	if pos := inputPosition(s.input); pos != 6 {
		t.Fatalf("cursor should be just past the newline (pos 6), got %d", pos)
	}
}

// TestShiftEnterInsertsNewline: in v2, modified Enter arrives as a real
// KeyPressMsg with modifier bits (Code: KeyEnter, Mod: ModShift). handleKey's
// kb(msg, "newline") matches it (String() == "shift+enter") and inserts a line
// break rather than sending the message.
func TestShiftEnterInsertsNewline(t *testing.T) {
	s := newInputSession(t, 60)
	s.input.SetValue("foo")
	setInputCursor(&s.input, len([]rune(s.input.Value()))) // cursor at end

	m, _ := s.Update(tea.KeyPressMsg{Code: tea.KeyEnter, Mod: tea.ModShift})
	if m != s {
		t.Fatalf("Update should return the same session")
	}
	if got := s.input.Value(); got != "foo\n" {
		t.Fatalf("Shift+Enter should insert a newline → %q, got %q", "foo\\n", got)
	}
}

// TestShiftEnterDoesNothingInModal: a Shift+Enter while a modal is open must
// not mutate the input (it has no meaning there).
func TestShiftEnterDoesNothingInModal(t *testing.T) {
	s := newInputSession(t, 60)
	s.input.SetValue("foo")
	s.openSettings() // any modal
	m, _ := s.Update(tea.KeyPressMsg{Code: tea.KeyEnter, Mod: tea.ModShift})
	if m != s {
		t.Fatalf("Update should return the same session")
	}
	if got := s.input.Value(); got != "foo" {
		t.Fatalf("shift+enter in a modal should not change input, got %q", got)
	}
}

// TestSentMultilineMessageKeepsBreaks: when a multi-line input is sent, the
// user block in the transcript renders each line on its own row (renderMarkdown
// splits on '\n'), so the sent message looks the same as the input box.
func TestSentMultilineMessageKeepsBreaks(t *testing.T) {
	s := newInputSession(t, 60)
	s.logUser("line1\nline2")

	// render the pushed user block the same way the transcript does
	b := s.blocks[len(s.blocks)-1]
	rendered := stripANSI(s.renderBlock(b, s.viewport.Width()))
	lines := strings.Split(rendered, "\n")

	// find the two source lines on distinct rows
	seen := map[string]bool{}
	for _, ln := range lines {
		if strings.Contains(ln, "line1") {
			seen["line1"] = true
		}
		if strings.Contains(ln, "line2") {
			seen["line2"] = true
		}
	}
	if !seen["line1"] || !seen["line2"] {
		t.Fatalf("sent multi-line message should keep both lines on separate rows; got:\n%s", rendered)
	}
	// they must NOT be on the same row (a single '\n' must not be collapsed to a space)
	for _, ln := range lines {
		if strings.Contains(ln, "line1") && strings.Contains(ln, "line2") {
			t.Fatalf("both lines collapsed onto one row: %q", ln)
		}
	}
}
