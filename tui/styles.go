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
	toolDiffAdded      = lipgloss.NewStyle().Foreground(lipgloss.Color(c.success))
	toolDiffRemoved    = lipgloss.NewStyle().Foreground(lipgloss.Color(c.err))
	toolDiffContext    = lipgloss.NewStyle().Foreground(lipgloss.Color(c.muted))
	toolDiffMeta       = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent))
	roToolNameStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent)).Bold(true) // read-only tools
	errOutStyle        = lipgloss.NewStyle().Foreground(lipgloss.Color(c.err))               // failed tool output text
	errRuleStyle       = lipgloss.NewStyle().Foreground(lipgloss.Color(c.err))               // failed tool left rule
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

// truncatePath front-truncates a filesystem path (keeps the last segments, the
// part that identifies the file) so a deep cwd fits the header without losing
// the current directory name.
func truncatePath(p string, maxRunes int) string {
	if maxRunes <= 3 {
		return p
	}
	r := []rune(p)
	if len(r) <= maxRunes {
		return p
	}
	return "…" + string(r[len(r)-(maxRunes-1):])
}

// ---------------------------------------------------------------------------
// Tool-call styling primitives
//
// toolKind mirrors core/tools.rs::classify so the TUI can tint a tool's name
// (and icon) by its approval kind: read-only tools render cyan, destructive
// amber. This is display-only; the core remains the approval authority.
// ---------------------------------------------------------------------------

type toolKindT int

const (
	kindReadOnly toolKindT = iota
	kindDestructive
)

// toolKindOf returns the approval kind of a tool, mirroring the core's
// classify() so a glance at the header reveals whether the call mutates state.
func toolKindOf(name string) toolKindT {
	switch name {
	case "read_file", "list_dir", "grep", "glob", "bulk_read", "todo_read",
		"diagnostics", "finish", "contact_supervisor", "intercom",
		"git_status", "git_diff", "git_log", "memory":
		return kindReadOnly
	default:
		return kindDestructive
	}
}

// toolNameStyleFor returns the name style tinted by kind: read-only → accent
// (cyan), destructive → tool (amber).
func toolNameStyleFor(name string) lipgloss.Style {
	if toolKindOf(name) == kindReadOnly {
		return roToolNameStyle
	}
	return toolNameStyle
}

// toolIcon returns a per-family glyph for a tool, used as the header marker.
// One icon per family (not per tool) keeps the transcript scannable without
// glyph noise.
func toolIcon(name string) string {
	switch name {
	case "bash":
		return "❯"
	case "read_file", "bulk_read", "list_dir", "grep", "glob":
		return "▤"
	case "write_file", "edit", "patch", "bulk_write", "bulk_edit":
		return "✎"
	case "git_status", "git_diff", "git_log", "git_add", "git_commit":
		return "⎇"
	case "todo_write", "todo_read":
		return "☑"
	case "diagnostics":
		return "⊕"
	case "fetch":
		return "↬"
	case "memory":
		return "❖"
	case "subagent", "spawn":
		return "◈"
	case "finish":
		return "✓"
	case "contact_supervisor", "intercom":
		return "✉"
	default:
		return "▸"
	}
}

// toolDisplayName maps wire names to friendlier header labels (e.g.
// git_status → "git status"). Falls back to the raw name.
func toolDisplayName(name string) string {
	switch name {
	case "git_status":
		return "git status"
	case "git_diff":
		return "git diff"
	case "git_log":
		return "git log"
	case "git_add":
		return "git add"
	case "git_commit":
		return "git commit"
	case "contact_supervisor":
		return "contact supervisor"
	}
	return name
}

// panelLines renders tool output with a left `│` rule (wrapped to fit), using
// the given rule + content styles. resultPanel selects dim/err styling by the
// `err` flag so a failed call's body reads red while scrolling.
func panelLines(output string, w int, rule, content lipgloss.Style) string {
	contentW := w - 3 // "│ " prefix + content
	if contentW < 2 {
		contentW = 2
	}
	r := rule.Render("│ ")
	wrapped := wrapPlain(output, contentW)
	var b strings.Builder
	for _, l := range strings.Split(wrapped, "\n") {
		b.WriteString(r)
		b.WriteString(content.Render(l))
		b.WriteByte('\n')
	}
	return strings.TrimRight(b.String(), "\n")
}
