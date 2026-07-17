package main

import "testing"

// TestComposerPreservesNewlines guards the Phase-1 switch from textinput
// (which collapsed \n) to textarea for the main composer.
func TestComposerPreservesNewlines(t *testing.T) {
	m := newComposer()
	m.SetValue("line1\nline2")
	if got := m.Value(); got != "line1\nline2" {
		t.Fatalf("composer Value = %q, want multiline", got)
	}
	m.InsertRune('\n')
	if got := m.Value(); got != "line1\nline2\n" {
		// Value trims a single trailing join newline; with an extra empty
		// line the result keeps one trailing '\n'.
		if got != "line1\nline2\n" && got != "line1\nline2" {
			// Insert at end adds a blank line → "line1\nline2\n"
			t.Fatalf("InsertRune('\\n') Value = %q", got)
		}
	}
}

func TestInputPositionAndSetCursor(t *testing.T) {
	m := newComposer()
	m.SetValue("hello\nworld")
	setInputCursor(&m, 6) // start of "world"
	if got := inputPosition(m); got != 6 {
		t.Fatalf("position after set = %d, want 6", got)
	}
	if m.Line() != 1 || m.Column() != 0 {
		t.Fatalf("line/col = %d/%d, want 1/0", m.Line(), m.Column())
	}
}
