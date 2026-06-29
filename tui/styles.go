package main

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// ---------------------------------------------------------------------------
// Color palette (Catppuccin Mocha-ish; renders well on truecolor + 256-color)
// ---------------------------------------------------------------------------

var c = struct {
	bg      string
	fg      string
	dim     string
	muted   string
	accent  string
	user    string
	assist  string
	tool    string
	success string
	warn    string
	err     string
}{
	bg:      "#1e1e2e",
	fg:      "#cdd6f4",
	dim:     "#6c7086",
	muted:   "#9399b2",
	accent:  "#74c7ec",
	user:    "#89b4fa",
	assist:  "#cdd6f4",
	tool:    "#f9e2af",
	success: "#a6e3a1",
	warn:    "#fab387",
	err:     "#f38ba8",
}

var (
	baseStyle          = lipgloss.NewStyle().Foreground(lipgloss.Color(c.fg))
	boldBaseStyle      = lipgloss.NewStyle().Foreground(lipgloss.Color(c.fg)).Bold(true)
	dimStyle           = lipgloss.NewStyle().Foreground(lipgloss.Color(c.dim))
	mutedStyle         = lipgloss.NewStyle().Foreground(lipgloss.Color(c.muted))
	accentStyle        = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent)).Bold(true)
	successStyle       = lipgloss.NewStyle().Foreground(lipgloss.Color(c.success))
	errStyle           = lipgloss.NewStyle().Foreground(lipgloss.Color(c.err))
	warnStyle          = lipgloss.NewStyle().Foreground(lipgloss.Color(c.warn)).Bold(true)
	assistantStyle     = lipgloss.NewStyle().Foreground(lipgloss.Color(c.assist))
	thinkStyle         = lipgloss.NewStyle().Foreground(lipgloss.Color(c.dim)).Italic(true)
	toolNameStyle      = lipgloss.NewStyle().Foreground(lipgloss.Color(c.tool)).Bold(true)
	toolDetailStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color(c.tool))
	resultStyle        = lipgloss.NewStyle().Foreground(lipgloss.Color(c.muted))
	headerStyle        = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent)).Bold(true)
	keyHintStyle       = lipgloss.NewStyle().Foreground(lipgloss.Color(c.dim))
	separatorStyle     = lipgloss.NewStyle().Foreground(lipgloss.Color(c.dim))
	inputPromptStyle   = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent))
	placeholderStyle   = lipgloss.NewStyle().Foreground(lipgloss.Color(c.dim))
	codeTextStyle      = lipgloss.NewStyle().Foreground(lipgloss.Color(c.fg))
	codeInlineStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color(c.tool))
	italicStyle        = lipgloss.NewStyle().Foreground(lipgloss.Color(c.fg)).Italic(true)
	linkStyle          = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent)).Underline(true)
	roleUserStyle      = lipgloss.NewStyle().Foreground(lipgloss.Color(c.user)).Bold(true)
	roleAssistantStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent)).Bold(true)
	roleThinkStyle     = lipgloss.NewStyle().Foreground(lipgloss.Color(c.dim)).Italic(true)
	roleToolStyle      = lipgloss.NewStyle().Foreground(lipgloss.Color(c.tool)).Bold(true)
	roleResultStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color(c.success)).Bold(true)
	roleErrorStyle     = lipgloss.NewStyle().Foreground(lipgloss.Color(c.err)).Bold(true)
	roleWarnStyle      = lipgloss.NewStyle().Foreground(lipgloss.Color(c.warn)).Bold(true)
	roleSuccessStyle   = lipgloss.NewStyle().Foreground(lipgloss.Color(c.success)).Bold(true)
)

// pillStyle returns a solid-background pill chip style for header tags.
// ponytail: no rounded border on pills — solid bg reads cleaner at small size.
func pillStyle(bg string) lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(c.bg)).
		Background(lipgloss.Color(bg)).
		Bold(true).
		Padding(0, 1)
}

// renderFlow applies a style to text line-by-line so lipgloss's multiline
// reflow (which pads short lines to the longest) never runs.
func renderFlow(text string, st lipgloss.Style) string {
	lines := strings.Split(text, "\n")
	for i, l := range lines {
		lines[i] = st.Render(l)
	}
	return strings.Join(lines, "\n")
}

// wrapPlain is a greedy word-wrap (breaks at spaces, hard-breaks long tokens).
// rune-counted, not display-width-aware: CJK/emoji wide chars may overflow.
// ponytail: swap for runewidth-based wrap if wide-char content misaligns boxes.
func wrapPlain(text string, w int) string {
	if w < 1 {
		w = 1
	}
	var lines []string
	for _, line := range strings.Split(text, "\n") {
		lines = append(lines, wrapLine(line, w)...)
	}
	return strings.Join(lines, "\n")
}

func wrapLine(line string, w int) []string {
	r := []rune(line)
	if len(r) == 0 {
		return []string{""}
	}
	var out []string
	for len(r) > 0 {
		end := w
		if end > len(r) {
			end = len(r)
		}
		if end < len(r) {
			// break at the last space within r[1:end] to keep words intact
			brk := -1
			for j := end - 1; j > 0; j-- {
				if r[j] == ' ' {
					brk = j
					break
				}
			}
			if brk > 0 {
				out = append(out, string(r[:brk]))
				r = r[brk+1:] // skip the space
				continue
			}
		}
		out = append(out, string(r[:end]))
		r = r[end:]
	}
	return out
}

// dimRule returns a dim horizontal rule of width w (used by markdown hr).
func dimRule(w int) string {
	if w < 1 {
		w = 1
	}
	return dimStyle.Render(strings.Repeat("─", w))
}

// truncate clips a string to maxRunes, appending "…" if it was shortened.
func truncate(s string, maxRunes int) string {
	if maxRunes <= 0 {
		return s
	}
	r := []rune(s)
	if len(r) <= maxRunes {
		return s
	}
	if maxRunes <= 1 {
		return "…"
	}
	return string(r[:maxRunes-1]) + "…"
}
