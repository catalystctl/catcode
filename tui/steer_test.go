package main

import (
	tea "github.com/charmbracelet/bubbletea"
	"testing"
)

func TestIsCtrlEnterUnknownCSI(t *testing.T) {
	// unknownCSISequenceMsg is an unexported []byte type in bubbletea; a plain
	// []byte has the same reflect shape (Slice of Uint8), so it exercises the
	// same path the real message would.
	cases := []struct {
		name string
		msg  tea.Msg
		want bool
	}{
		{"kitty_csi_u", []byte("\x1b[13;5u"), true},
		{"xterm_modifyother", []byte("\x1b[27;5;13~"), true},
		{"plain_enter_cr", []byte("\r"), false},
		{"ctrl_n", []byte("\x0e"), false},
		{"unrelated_csi", []byte("\x1b[5;5~"), false},
		{"empty", []byte(""), false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := isCtrlEnterUnknownCSI(c.msg); got != c.want {
				t.Errorf("isCtrlEnterUnknownCSI(%q) = %v, want %v", c.msg, got, c.want)
			}
		})
	}

	// Non-slice messages must never match.
	if isCtrlEnterUnknownCSI(tea.KeyMsg{Type: tea.KeyEnter}) {
		t.Error("KeyMsg should not match")
	}
}
