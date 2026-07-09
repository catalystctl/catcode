package main

import (
	"bytes"
	"fmt"
	"reflect"
	"strconv"
	"strings"
	"time"
	"unsafe"

	"github.com/atotto/clipboard"
	"github.com/charmbracelet/bubbles/runeutil"
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

// enableMultilineInput swaps textinput's rune sanitizer for one that PRESERVES
// newlines, so Shift+Enter line breaks and pasted multi-line text survive.
//
// bubbles' textinput assumes single-line input: its lazily-built sanitizer is
// runeutil.NewSanitizer(runeutil.ReplaceTabs(" "), runeutil.ReplaceNewlines(" "))
// which collapses every '\n' — whether typed or pasted — into a space, making
// multi-line composition impossible. The sanitizer is cached in an unexported
// `rsan` field with no public setter, so we set it once (via unsafe) right
// after New() to runeutil.NewSanitizer() with no options: that keeps '\n'
// (only '\r'→'\n', tabs→4 spaces, and other control chars are stripped).
//
// Guarded: if the internal field layout ever changes we skip and fall back to
// single-line behavior rather than crash. bubbles is pinned at v1.0.0.
func (s *session) enableMultilineInput() {
	v := reflect.ValueOf(&s.input).Elem()
	f := v.FieldByName("rsan")
	if !f.IsValid() || !f.CanAddr() {
		return // field renamed/removed in a future bubbles — degrade to single-line
	}
	*(*runeutil.Sanitizer)(unsafe.Pointer(f.UnsafeAddr())) = runeutil.NewSanitizer()
}

// isModifiedEnterCSI reports whether msg is bubbletea's unrecognized-CSI
// message carrying a modified-Enter sequence for the given modifier code
// (Kitty `\x1b[13;<mod>u`, xterm modifyOtherKeys `\x1b[27;<mod>;13~`).
// bubbletea v1.3's Key type carries no modifier bits, so modified-Enter can't
// arrive as a tea.KeyMsg; terminals that send a distinct CSI for it (Kitty,
// xterm modifyOtherKeys) surface it as `unknownCSISequenceMsg` (an unexported
// []byte type), which we reach by reflection. Returns false for everything else.
// Modifier codes: 2 = shift, 5 = ctrl.
func isModifiedEnterCSI(msg tea.Msg, mod byte) bool {
	v := reflect.ValueOf(msg)
	if v.Kind() != reflect.Slice || v.Type().Elem().Kind() != reflect.Uint8 {
		return false
	}
	b := v.Bytes()
	return bytes.Equal(b, []byte(fmt.Sprintf("\x1b[13;%du", mod))) ||
		bytes.Equal(b, []byte(fmt.Sprintf("\x1b[27;%d;13~", mod)))
}

// isCtrlEnterUnknownCSI reports whether msg is a Ctrl+Enter modified-Enter CSI.
func isCtrlEnterUnknownCSI(msg tea.Msg) bool { return isModifiedEnterCSI(msg, 5) }

// isShiftEnterUnknownCSI reports whether msg is a Shift+Enter modified-Enter CSI.
func isShiftEnterUnknownCSI(msg tea.Msg) bool { return isModifiedEnterCSI(msg, 2) }

// ctrlLetterKeyFromModifiedCSI converts the non-Enter CSI-u / modifyOtherKeys
// sequences produced by terminals after we request enhanced keyboard reporting
// back into ordinary Bubble Tea Ctrl+letter KeyMsgs. This lets us enable the
// more reliable protocols needed for Ctrl+Enter / Shift+Enter without breaking
// existing bindings like Ctrl+C, Ctrl+P, Ctrl+K, Ctrl+T, and Ctrl+O.
//
// Supported forms:
//   - Kitty progressive keyboard: ESC [ <codepoint> ; <mods> u
//   - xterm modifyOtherKeys level 2: ESC [ 27 ; <mods> ; <codepoint> ~
//
// Modifier encoding is the same in both protocols: 1 + bitmask where ctrl is
// bit 4 (so ctrl alone is 5, ctrl+shift is 6, ctrl+alt is 7, ...). We only
// synthesize Ctrl+A..Ctrl+Z; other enhanced keys either have dedicated handling
// (Enter above) or should remain ignored rather than guessed.
func ctrlLetterKeyFromModifiedCSI(msg tea.Msg) (tea.KeyMsg, bool) {
	v := reflect.ValueOf(msg)
	if v.Kind() != reflect.Slice || v.Type().Elem().Kind() != reflect.Uint8 {
		return tea.KeyMsg{}, false
	}
	s := string(v.Bytes())
	if !strings.HasPrefix(s, "\x1b[") {
		return tea.KeyMsg{}, false
	}

	var code, mods int
	var err error
	body := strings.TrimPrefix(s, "\x1b[")
	switch {
	case strings.HasSuffix(body, "u"):
		parts := strings.Split(strings.TrimSuffix(body, "u"), ";")
		if len(parts) != 2 {
			return tea.KeyMsg{}, false
		}
		code, err = strconv.Atoi(parts[0])
		if err != nil {
			return tea.KeyMsg{}, false
		}
		mods, err = strconv.Atoi(parts[1])
		if err != nil {
			return tea.KeyMsg{}, false
		}
	case strings.HasSuffix(body, "~"):
		parts := strings.Split(strings.TrimSuffix(body, "~"), ";")
		if len(parts) != 3 || parts[0] != "27" {
			return tea.KeyMsg{}, false
		}
		mods, err = strconv.Atoi(parts[1])
		if err != nil {
			return tea.KeyMsg{}, false
		}
		code, err = strconv.Atoi(parts[2])
		if err != nil {
			return tea.KeyMsg{}, false
		}
	default:
		return tea.KeyMsg{}, false
	}

	if mods&4 == 0 { // ctrl bit not set
		return tea.KeyMsg{}, false
	}
	if code >= 'A' && code <= 'Z' {
		code += 'a' - 'A'
	}
	if code < 'a' || code > 'z' {
		return tea.KeyMsg{}, false
	}
	return tea.KeyMsg{Type: tea.KeyType(code - 'a' + 1)}, true
}

// SS3 Enter (\x1bOM) — the form VS Code's and Konsole's terminals send for
// Shift+Enter when no keyboard protocol is engaged. bubbletea v1.3.10 has no
// \x1bOM mapping (it only knows \x1bOA-D / \x1bOP-S) and \x1bOM is NOT a CSI
// (\x1b[…), so it never becomes the `unknownCSISequenceMsg` the CSI helpers
// above catch. detectOneMsg instead consumes ESC as an Alt-prefix and emits
// TWO separate tea.KeyMsg values: first {KeyRunes, Runes:['O'], Alt:true},
// then {KeyRunes, Runes:['M']} — both reach handleKey and (without the buffer
// in handleKey) get inserted into the input as "OM".
//
// We can't catch \x1bOM as a single message, so handleKey buffers it: the
// Alt-'O' lead sets s.pendingSS3 (and is consumed); the trailing 'M' then
// resolves it as a Shift+Enter → insertNewline. These two helpers classify
// each half. (When modifyOtherKeys is engaged the terminal sends a proper CSI
// instead, so this buffer only fires for the legacy \x1bOM form.)

// isSS3EnterLead reports whether msg is the Alt-'O' KeyRunes that bubbletea
// emits as the first half of an \x1bOM (SS3 keypad-Enter) sequence.
func isSS3EnterLead(msg tea.KeyMsg) bool {
	return msg.Alt && msg.Type == tea.KeyRunes && len(msg.Runes) == 1 && msg.Runes[0] == 'O'
}

// isSS3EnterRune reports whether msg is the trailing plain-'M' KeyRunes that
// follows the Alt-'O' lead to complete an \x1bOM sequence.
func isSS3EnterRune(msg tea.KeyMsg) bool {
	return !msg.Alt && msg.Type == tea.KeyRunes && len(msg.Runes) == 1 && msg.Runes[0] == 'M'
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
