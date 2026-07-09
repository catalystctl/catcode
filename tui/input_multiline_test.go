package main

import (
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
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
	// top + 2 content rows + bottom
	if len(lines) != 4 {
		t.Fatalf("two-line input should render 4 box rows (got %d):\n%s", len(lines), box)
	}
	// both source lines must appear on their own bordered rows, neither dropped
	if !strings.Contains(lines[1], "line1") {
		t.Fatalf("first line missing from row 1: %q\n%s", lines[1], box)
	}
	if !strings.Contains(lines[2], "line2") {
		t.Fatalf("second line missing from row 2: %q\n%s", lines[2], box)
	}
	for i := 1; i < 3; i++ {
		if !strings.HasPrefix(lines[i], "│") || !strings.HasSuffix(lines[i], "│") {
			t.Fatalf("content row %d missing side borders: %q", i, lines[i])
		}
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
	for _, ln := range strings.Split(box, "\n")[1 : len(strings.Split(box, "\n"))-1] {
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
	s.input.SetCursor(len([]rune(s.input.Value()))) // cursor at very end
	s.layout()
	box := stripANSI(s.renderInputBox())
	lines := strings.Split(box, "\n")
	// top + "hello" + blank + bottom = 4 rows
	if len(lines) != 4 {
		t.Fatalf("trailing newline should render a blank row (got %d rows):\n%s", len(lines), box)
	}
	// row 3 is a blank boxed line: side borders with only spaces between them.
	blank := strings.Trim(lines[2], "│ ")
	if blank != "" {
		t.Fatalf("row 3 should be a blank boxed line, got %q", lines[2])
	}
}

// TestInsertNewlineAtCursor inserts a line break at the cursor and leaves the
// cursor on the new (second) line.
func TestInsertNewlineAtCursor(t *testing.T) {
	s := newInputSession(t, 60)
	s.input.SetValue("hello world")
	s.input.SetCursor(5) // between "hello" and " world"
	s.insertNewline()

	if got := s.input.Value(); got != "hello\n world" {
		t.Fatalf("value after insertNewline = %q, want %q", got, "hello\n world")
	}
	if pos := s.input.Position(); pos != 6 {
		t.Fatalf("cursor should be just past the newline (pos 6), got %d", pos)
	}
}

// TestShiftEnterCSIInsertsNewline simulates the Kitty/xterm Shift+Enter CSI
// arriving as bubbletea's unrecognized-CSI message and verifies it inserts a
// line break (rather than, say, sending the message).
func TestShiftEnterCSIInsertsNewline(t *testing.T) {
	s := newInputSession(t, 60)
	s.input.SetValue("foo")
	s.input.SetCursor(len(s.input.Value())) // cursor at end

	// A plain []byte has the same reflect shape as bubbletea's unexported
	// unknownCSISequenceMsg, so Update()'s default arm reaches it via reflection.
	for _, csi := range [][]byte{[]byte("\x1b[13;2u"), []byte("\x1b[27;2;13~")} {
		s.input.Reset()
		s.input.SetValue("foo")
		s.input.SetCursor(len(s.input.Value()))
		m, _ := s.Update(csi)
		if m != s {
			t.Fatalf("Update should return the same session")
		}
		if got := s.input.Value(); got != "foo\n" {
			t.Fatalf("CSI %q should insert a newline → %q, got %q", csi, "foo\\n", got)
		}
	}
}

// TestShiftEnterCSIDoesNothingInModal: a Shift+Enter while a modal is open must
// not mutate the input (it has no meaning there).
func TestShiftEnterCSIDoesNothingInModal(t *testing.T) {
	s := newInputSession(t, 60)
	s.input.SetValue("foo")
	s.openSettings() // any modal
	m, _ := s.Update([]byte("\x1b[13;2u"))
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
	rendered := stripANSI(s.renderBlock(b, s.viewport.Width))
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
func TestIsShiftEnterUnknownCSI(t *testing.T) {
	cases := []struct {
		name string
		msg  tea.Msg
		want bool
	}{
		{"kitty_csi_u", []byte("\x1b[13;2u"), true},
		{"xterm_modifyother", []byte("\x1b[27;2;13~"), true},
		{"ctrl_enter_kitty", []byte("\x1b[13;5u"), false},
		{"ctrl_enter_xterm", []byte("\x1b[27;5;13~"), false},
		{"plain_enter_cr", []byte("\r"), false},
		{"empty", []byte(""), false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := isShiftEnterUnknownCSI(c.msg); got != c.want {
				t.Errorf("isShiftEnterUnknownCSI(%q) = %v, want %v", c.msg, got, c.want)
			}
		})
	}
	if isShiftEnterUnknownCSI(tea.KeyMsg{Type: tea.KeyEnter}) {
		t.Error("KeyMsg should not match")
	}
}

// TestSS3EnterClassification covers the two halves of an \x1bOM (SS3
// keypad-Enter) sequence as bubbletea v1.3.10 splits it: an Alt-'O' lead and
// a plain 'M' trailer.
func TestSS3EnterClassification(t *testing.T) {
	lead := tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'O'}, Alt: true}
	trail := tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'M'}}
	if !isSS3EnterLead(lead) {
		t.Error("Alt+'O' KeyRunes should be the SS3 Enter lead")
	}
	if isSS3EnterLead(trail) {
		t.Error("plain 'M' should not be the SS3 Enter lead")
	}
	if !isSS3EnterRune(trail) {
		t.Error("plain 'M' KeyRunes should be the SS3 Enter trailing rune")
	}
	if isSS3EnterRune(lead) {
		t.Error("Alt+'O' should not be the SS3 Enter trailing rune")
	}
	// plain 'O' (no Alt) is NOT the lead — the ESC prefix is what sets Alt.
	if isSS3EnterLead(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'O'}}) {
		t.Error("plain 'O' without Alt should not be the SS3 Enter lead")
	}
	if isSS3EnterLead(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'A'}, Alt: true}) {
		t.Error("Alt+'A' should not be the SS3 Enter lead")
	}
}

// TestShiftEnterSS3InsertsNewline: VS Code's and Konsole's terminals send
// \x1bOM (SS3 keypad-Enter) for Shift+Enter. bubbletea v1.3.10 has no \x1bOM
// mapping and it's not a CSI, so detectOneMsg splits it into two KeyMsgs —
// Alt-'O' then 'M'. handleKey must buffer the lead and resolve the trailing
// 'M' as a Shift+Enter → insertNewline (rather than leaking "OM" into input).
func TestShiftEnterSS3InsertsNewline(t *testing.T) {
	s := newInputSession(t, 60)
	s.input.SetValue("foo")
	s.input.SetCursor(len(s.input.Value()))

	// First half of \x1bOM: Alt-'O'. Consumed (sets pendingSS3), nothing inserted.
	m, _ := s.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'O'}, Alt: true})
	if m != s {
		t.Fatalf("Update should return the same session")
	}
	if got := s.input.Value(); got != "foo" {
		t.Fatalf("Alt-'O' lead should not insert anything, got %q", got)
	}
	// Second half: plain 'M'. Resolves as Shift+Enter → insertNewline.
	m, _ = s.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'M'}})
	if m != s {
		t.Fatalf("Update should return the same session")
	}
	if got := s.input.Value(); got != "foo\n" {
		t.Fatalf("trailing 'M' of \\x1bOM should insert a newline → %q, got %q", "foo\\n", got)
	}
	if s.pendingSS3 {
		t.Fatalf("pendingSS3 should be cleared after resolving")
	}
}

// TestSS3LeadFollowedByOtherKeyDropsO: if the Alt-'O' lead is NOT followed by
// 'M' (a spurious Alt-'O', rare), the buffered 'O' is dropped and the
// following key is handled normally — pendingSS3 must not get stuck.
func TestSS3LeadFollowedByOtherKeyDropsO(t *testing.T) {
	s := newInputSession(t, 60)
	s.input.SetValue("foo")
	s.input.SetCursor(len(s.input.Value()))

	s.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'O'}, Alt: true}) // lead
	m, _ := s.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'x'}})
	if m != s {
		t.Fatalf("Update should return the same session")
	}
	if s.pendingSS3 {
		t.Fatalf("pendingSS3 should be cleared after a non-'M' follow-up")
	}
	// 'O' is dropped (Alt-'O' isn't a real chat input); 'x' is inserted normally.
	if got := s.input.Value(); got != "foox" {
		t.Fatalf("after spurious Alt-'O' + 'x', input should be %q, got %q", "foox", got)
	}
}
