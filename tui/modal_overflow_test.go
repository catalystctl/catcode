package main

import (
	"fmt"
	"strings"
	"testing"

	"charm.land/lipgloss/v2"
)

// makeSessionList builds n synthetic session entries for modal tests.
func makeSessionList(n int) []sessionEntry {
	out := make([]sessionEntry, n)
	for i := 0; i < n; i++ {
		out[i] = sessionEntry{
			Name:     fmt.Sprintf("session-%d", i),
			Path:     fmt.Sprintf("/tmp/session-%d.jsonl", i),
			Title:    fmt.Sprintf("A moderately long session title number %d", i),
			Messages: i + 1,
			Mtime:    uint64(1700000000 + i),
			Current:  i == 0,
		}
	}
	return out
}

// TestSessionModalFitsTerminal reproduces the "session list too long overflows
// the viewable space" bug: with many sessions the picker modal must never grow
// taller than the terminal, regardless of list length or terminal height.
func TestSessionModalFitsTerminal(t *testing.T) {
	for _, h := range []int{10, 12, 16, 20, 24, 30, 40, 50} {
		for _, n := range []int{0, 1, 5, 15, 30, 100, 500} {
			s := initialSession()
			s.ready = true
			s.width, s.height = 80, h
			s.sessionList = makeSessionList(n)
			s.openSessionsPicker()
			s.layout()

			box := s.renderModalBody()
			got := lipgloss.Height(box)
			if got > s.height {
				t.Errorf("height=%d sessions=%d: modal height %d overflows terminal %d\n%s",
					h, n, got, s.height, stripANSI(box))
			}
			// The full overlay canvas must also be exactly terminal-sized.
			overlay := s.renderModalOverlay("base")
			if oh := lipgloss.Height(overlay); oh != s.height {
				t.Errorf("height=%d sessions=%d: overlay height %d != terminal %d",
					h, n, oh, s.height)
			}
		}
	}
}

// TestSessionModalRowsAreSingleLine confirms the root cause stays fixed: every
// visible session row renders as exactly one physical line (no wrapping), so
// the scroll window's line budget matches the rendered height.
func TestSessionModalRowsAreSingleLine(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.sessionList = makeSessionList(100)
	s.openSessionsPicker()
	s.layout()
	box := s.renderModalBody()
	for i, line := range strings.Split(box, "\n") {
		if w := lipgloss.Width(line); w > s.width {
			t.Errorf("row %d width %d overflows terminal width %d: %q", i, w, s.width, stripANSI(line))
		}
	}
}

// TestSessionModalPreservesDescription checks that the msg-count/time
// description survives on long-title rows: the row fits one line by truncating
// the title, not by chopping the description.
func TestSessionModalPreservesDescription(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.sessionList = []sessionEntry{{
		Name: "x", Path: "/tmp/x.jsonl",
		Title:    "This is a very long session title that used to wrap and overflow the terminal",
		Messages: 42, Mtime: 1700000000,
	}}
	s.openSessionsPicker()
	s.layout()
	box := stripANSI(s.renderModalBody())
	if !strings.Contains(box, "42 msgs") {
		t.Errorf("description (msg count) should be preserved on long-title rows; got:\n%s", box)
	}
}

func TestTruncateFit(t *testing.T) {
	cases := []struct {
		in   string
		n    int
		want string
	}{
		{"hello", 10, "hello"}, // no cut
		{"hello", 5, "hello"},  // exact
		{"hello", 4, "hel…"},   // cut, ellipsis counts toward n
		{"hello", 1, "…"},      // single rune -> ellipsis only
		{"hello", 0, ""},       // no budget
		{"hello", -1, ""},      // negative budget
		{"", 5, ""},            // empty
	}
	for _, c := range cases {
		if got := truncateFit(c.in, c.n); got != c.want {
			t.Errorf("truncateFit(%q,%d) = %q, want %q", c.in, c.n, got, c.want)
		}
	}
	// result never exceeds n runes
	for _, n := range []int{1, 2, 3, 5, 9} {
		got := []rune(truncateFit("a moderately long string", n))
		if len(got) > n {
			t.Errorf("truncateFit result %q exceeds %d runes", string(got), n)
		}
	}
}

func TestFitListRow(t *testing.T) {
	// Wide row: label + desc both fit.
	r := stripANSI(fitListRow("  ", "hello", "5 msgs", 2, 40, ""))
	if !strings.Contains(r, "hello") || !strings.Contains(r, "5 msgs") {
		t.Errorf("wide row should keep label+desc; got %q", r)
	}
	// Narrow row: label truncated, desc preserved.
	r = stripANSI(fitListRow("  ", "a very long title here", "12 msgs", 2, 20, ""))
	if !strings.Contains(r, "12 msgs") {
		t.Errorf("narrow row should preserve desc; got %q", r)
	}
	if strings.Contains(r, "a very long title here") {
		t.Errorf("narrow row should truncate the label; got %q", r)
	}
	// Result must not exceed the width budget.
	for _, w := range []int{5, 10, 20, 40} {
		got := lipgloss.Width(fitListRow("  ", "some long long long label", "99 msgs", 2, w, ""))
		if got > w {
			t.Errorf("fitListRow width %d > budget %d", got, w)
		}
	}
}
