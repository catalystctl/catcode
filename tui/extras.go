package main

import (
	"fmt"
	"strings"
	"time"

	tea "charm.land/bubbletea/v2"
	"github.com/atotto/clipboard"
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
		// Fresh copy so the dropped prefix (and large pastes it holds) can be
		// GC'd — slicing alone keeps the old backing array alive (D-009).
		kept := make([]string, historyMax)
		copy(kept, s.history[len(s.history)-historyMax:])
		s.history = kept
	}
	s.histIdx = len(s.history)
}

// insertNewline inserts a literal line break at the composer cursor so
// Shift+Enter builds a multi-line message. Terminates any active @-mention.
func (s *session) insertNewline() {
	s.input.InsertRune('\n')
	s.evalMention()
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

// historyRecallAllowed gates Up/Down history against multi-line editing.
// dir -1 = Up (history_prev): only when input is empty or cursor is on the
// first line. dir +1 = Down: empty or cursor on the last line.
func (s *session) historyRecallAllowed(dir int) bool {
	if s.input.Value() == "" {
		return true
	}
	if dir < 0 {
		return s.input.Line() == 0
	}
	return s.input.Line() >= s.input.LineCount()-1
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
				if err := clipboard.WriteAll(text); err == nil {
					s.logSuccess("copied last reply to clipboard")
					return nil
				} else {
					// Headless/SSH sessions commonly have no host clipboard. OSC 52
					// targets the user's terminal clipboard and is the most useful
					// fallback, but support is terminal-controlled and cannot be
					// acknowledged, so report that distinction honestly.
					s.logWarn("system clipboard unavailable; sent copy request to terminal (OSC 52)")
					return writeOSC52Cmd(text)
				}
			}
		}
	}
	s.logError("no assistant reply to copy")
	return nil
}

// ---------------------------------------------------------------------------
// Accessors
// ---------------------------------------------------------------------------

// approvalMode returns the current approval mode (settings-backed, updated from
// core events). Never returns blank — falls back to the persisted setting, then
// destructive. Ignores "<kind>:always" escalation strings that used to leak
// into approvalModeStr and display as "destructive".
func (s *session) approvalMode() string {
	if s.approvalModeStr != "" && !strings.Contains(s.approvalModeStr, ":") {
		return normalizeApproval(s.approvalModeStr)
	}
	if s.settings != nil && s.settings.Approval != "" {
		return normalizeApproval(s.settings.Approval)
	}
	return "destructive"
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
