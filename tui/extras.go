package main

import (
	"bytes"
	"fmt"
	"reflect"
	"strings"
	"time"

	"github.com/atotto/clipboard"
	tea "github.com/charmbracelet/bubbletea"
)

// ---------------------------------------------------------------------------
// Input history (up/down recall of past prompts)
// ---------------------------------------------------------------------------

const historyMax = 100

func (s *session) pushHistory(line string) {
	if strings.TrimSpace(line) == "" {
		return
	}
	// Drop leading slash-only echoes from history? Keep them — useful to recall.
	s.history = append(s.history, line)
	if len(s.history) > historyMax {
		s.history = s.history[len(s.history)-historyMax:]
	}
	s.histIdx = len(s.history)
}

// isCtrlEnterUnknownCSI reports whether msg is bubbletea's unrecognized-CSII
// message carrying a Ctrl+Enter sequence. bubbletea v1.3's Key type carries no
// modifier bits, so modified-Enter can't arrive as a tea.KeyMsg; terminals that
// send a distinct CSI for Ctrl+Enter (Kitty `\x1b[13;5u`, xterm modifyOtherKeys
// `\x1b[27;5;13~`) surface it as `unknownCSISequenceMsg` (an unexported []byte
// type), which we reach by reflection. Returns false for everything else.
func isCtrlEnterUnknownCSI(msg tea.Msg) bool {
	v := reflect.ValueOf(msg)
	if v.Kind() != reflect.Slice || v.Type().Elem().Kind() != reflect.Uint8 {
		return false
	}
	b := v.Bytes()
	return bytes.Equal(b, []byte("\x1b[13;5u")) ||
		bytes.Equal(b, []byte("\x1b[27;5;13~"))
}

func (s *session) recallHistory(dir int) string {
	if len(s.history) == 0 {
		return ""
	}
	s.histIdx += dir
	if s.histIdx < 0 {
		s.histIdx = 0
	}
	if s.histIdx > len(s.history) {
		s.histIdx = len(s.history)
	}
	if s.histIdx == len(s.history) {
		return ""
	}
	return s.history[s.histIdx]
}

// ---------------------------------------------------------------------------
// Clipboard: copy the last assistant block's text.
// ---------------------------------------------------------------------------

func (s *session) copyLastAssistant() tea.Cmd {
	for i := len(s.blocks) - 1; i >= 0; i-- {
		b := s.blocks[i]
		if b.kind == blkAssistant && b.text.Len() > 0 {
			text := strings.TrimSpace(b.text.String())
			if text != "" {
				_ = clipboard.WriteAll(text)
				s.logSuccess("copied last reply to clipboard")
				return nil
			}
		}
	}
	s.logError("no assistant reply to copy")
	return nil
}

// ---------------------------------------------------------------------------
// Accessors
// ---------------------------------------------------------------------------

// approvalMode returns the current approval mode tracked from core events.
func (s *session) approvalMode() string {
	if s.approvalModeStr == "" {
		return "destructive"
	}
	return s.approvalModeStr
}

// ---------------------------------------------------------------------------
// Session picker helpers
// ---------------------------------------------------------------------------

// formatMtime renders a session's last-modified time as a short relative
// label (e.g. "3h ago"); falls back to a calendar date for older sessions or
// clock skew.
func formatMtime(mtime uint64) string {
	if mtime == 0 {
		return "—"
	}
	t := time.Unix(int64(mtime), 0)
	d := time.Since(t)
	switch {
	case d < -time.Hour:
		return t.Format("Jan 2 15:04")
	case d < time.Minute:
		return "just now"
	case d < time.Hour:
		return fmt.Sprintf("%dm ago", int(d.Minutes()))
	case d < 24*time.Hour:
		return fmt.Sprintf("%dh ago", int(d.Hours()))
	default:
		return t.Format("Jan 2")
	}
}

// truncateRunes caps a string to n runes, appending an ellipsis if cut.
func truncateRunes(s string, n int) string {
	r := []rune(s)
	if len(r) <= n {
		return s
	}
	return string(r[:n]) + "…"
}

// truncateFit truncates s to at most n runes, appending "…" when cut. The
// ellipsis counts toward n, so the result never exceeds n runes; n <= 0 → "".
// Use this (instead of truncateRunes) when the result must fit a fixed column
// budget, e.g. a single-line list row inside a bordered modal.
func truncateFit(s string, n int) string {
	if n <= 0 {
		return ""
	}
	r := []rune(s)
	if len(r) <= n {
		return s
	}
	if n == 1 {
		return "…"
	}
	return string(r[:n-1]) + "…"
}
