package main

import (
	"fmt"
	"reflect"
	"strings"
	"time"
	"unsafe"

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
		s.history = s.history[len(s.history)-historyMax:]
	}
	s.histIdx = len(s.history)
}

// enableMultilineInput swaps textinput's rune sanitizer for one that PRESERVES
// newlines, so Shift+Enter line breaks and pasted multi-line text survive.
//
// bubbles' textinput assumes single-line input: its lazily-built sanitizer
// collapses every '\n' — whether typed or pasted — into a space, making
// multi-line composition impossible. The sanitizer is cached in an unexported
// `rsan` field with no public setter, so we set it once right after New() to a
// passthrough that keeps runes unchanged (so '\n' survives).
//
// bubbles v2 moved runeutil to an internal package, so we can't construct its
// Sanitizer directly; instead we install our own type implementing the same
// single-method interface (Sanitize([]rune) []rune). Guarded: if the field
// layout ever changes we skip and fall back to single-line rather than crash.
type passthroughSanitizer struct{}

func (passthroughSanitizer) Sanitize(runes []rune) []rune { return runes }

func (s *session) enableMultilineInput() {
	v := reflect.ValueOf(&s.input).Elem()
	f := v.FieldByName("rsan")
	if !f.IsValid() || !f.CanAddr() {
		return // field renamed/removed in a future bubbles — degrade to single-line
	}
	// rsan is unexported: use NewAt to obtain a settable reference, then assign
	// our passthrough sanitizer. f.Type() is the (internal) interface type;
	// passthroughSanitizer satisfies it via Go's structural interface typing.
	reflect.NewAt(f.Type(), unsafe.Pointer(f.UnsafeAddr())).Elem().
		Set(reflect.ValueOf(passthroughSanitizer{}))
}

// insertNewline inserts a literal line break ('\n') at the textinput cursor so
// Shift+Enter builds a multi-line message. Mirrors acceptMention's
// SetValue+SetCursor pattern: textinput.SetValue moves the cursor to the end,
// so we restore it to just past the inserted newline. A newline also terminates
// any active @-mention token, so evalMention closes the flyout.
func (s *session) insertNewline() {
	val := s.input.Value()
	pos := s.input.Position()
	r := []rune(val)
	if pos < 0 {
		pos = 0
	}
	if pos > len(r) {
		pos = len(r)
	}
	s.input.SetValue(string(r[:pos]) + "\n" + string(r[pos:]))
	s.input.SetCursor(pos + 1)
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

// approvalMode returns the current approval mode (settings-backed, updated from
// core events). Never returns blank — falls back to the persisted setting, then
// destructive.
func (s *session) approvalMode() string {
	if s.approvalModeStr != "" {
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
