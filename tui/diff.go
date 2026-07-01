package main

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// renderDiffPanel renders a unified-diff string as a colorized panel with a dim
// left `│` rule, mirroring renderOutputPanel's truncation + expand semantics:
// the first headLines raw diff lines are shown unless `expanded`; a dim hint
// line offers the ctrl+o toggle. A one-line dim file header is extracted from a
// `+++ b/<path>` marker when present (always shown, not counted toward the
// limit).
//
// Each diff line is classified by its leading marker and styled via the theme:
//   - (not "+++ ")  -> added   (green)
//   - (not "--- ")  -> removed (red)
//     "@@", "--- ", "+++ ", "diff --git", "\ " (no-newline marker) -> meta (cyan)
//     everything else -> context (dim)
//
// Lines are wrapped to the panel width on their RAW text before styling so the
// per-line style's ANSI escapes can't corrupt the rune-counted wrap.
func renderDiffPanel(diff string, expanded bool, w int) string {
	const headLines = 3
	diff = strings.TrimSpace(diff)
	if diff == "" {
		return ""
	}
	rawLines := strings.Split(diff, "\n")

	// Optional one-line file header drawn from a "+++ b/<path>" marker.
	header := ""
	for _, l := range rawLines {
		if strings.HasPrefix(l, "+++ ") {
			p := strings.TrimSpace(strings.TrimPrefix(l, "+++ "))
			p = strings.TrimPrefix(p, "b/")
			if p != "" && p != "/dev/null" {
				header = p
			}
			break
		}
	}

	contentW := w - 3 // "│ " prefix + content
	if contentW < 2 {
		contentW = 2
	}
	rule := dimStyle.Render("│ ")

	// emitRow wraps a raw line to the panel width and writes a rule + styled
	// piece per wrapped row. Wrapping runs on the raw text (before styling) so
	// the style's ANSI escapes can't skew the rune-counted wrap.
	emitRow := func(line string, st lipgloss.Style, b *strings.Builder) {
		for _, piece := range wrapLine(line, contentW) {
			b.WriteString(rule)
			b.WriteString(st.Render(piece))
			b.WriteByte('\n')
		}
	}

	// panelBody renders the given raw diff lines (already truncated if needed)
	// plus the optional file header, as rule-prefixed rows.
	panelBody := func(lines []string) string {
		var b strings.Builder
		if header != "" {
			emitRow("◆ "+header, dimStyle, &b)
		}
		for _, l := range lines {
			emitRow(l, diffLineStyle(l), &b)
		}
		return strings.TrimRight(b.String(), "\n")
	}

	// Truncation mirrors renderOutputPanel: first headLines raw lines unless
	// expanded, with a dim hint offering the ctrl+o toggle.
	if len(rawLines) > headLines && !expanded {
		more := len(rawLines) - headLines
		panel := panelBody(rawLines[:headLines])
		hint := dimStyle.Italic(true).Render(
			fmt.Sprintf("│ … +%d line%s  (ctrl+o expand)", more, pluralS(more)))
		return panel + "\n" + hint
	}
	panel := panelBody(rawLines)
	if len(rawLines) > headLines && expanded {
		panel += "\n" + dimStyle.Italic(true).Render("│ (ctrl+o collapse)")
	}
	return panel
}

// diffLineStyle classifies a unified-diff line by its leading marker and
// returns the matching theme style. The meta markers ("+++ ", "--- ", "@@",
// "diff --git", "\ ") are checked before the bare "+"/"-" cases so a file
// header line is never misclassified as an added/removed line.
func diffLineStyle(line string) lipgloss.Style {
	switch {
	case strings.HasPrefix(line, "+++ "),
		strings.HasPrefix(line, "--- "),
		strings.HasPrefix(line, "diff --git"),
		strings.HasPrefix(line, "@@"),
		strings.HasPrefix(line, "\\ "):
		return toolDiffMeta
	case strings.HasPrefix(line, "+"):
		return toolDiffAdded
	case strings.HasPrefix(line, "-"):
		return toolDiffRemoved
	default:
		return toolDiffContext
	}
}
