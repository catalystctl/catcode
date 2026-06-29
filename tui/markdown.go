package main

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// ---------------------------------------------------------------------------
// Lightweight markdown renderer
//
// Handles the subset that shows up in coding-agent replies: fenced code
// blocks, inline `code`, **bold**, *italic* / _italic_, [text](url) links,
// ATX headings, unordered/ordered lists, blockquotes, and hr rules.
//
// Inline formatting + wrapping is done segment-aware so a styled word carries
// its colour across line breaks. Code blocks are never reflowed (preserves
// formatting); over-long code lines are hard-truncated with "…".
//
// ponytail: not a full CommonMark parser — no tables, no nested lists, no
// setext headings. Add when a real payload needs them; chat replies rarely do.
// ---------------------------------------------------------------------------

// renderMarkdown turns raw markdown into a width-aware styled string.
func renderMarkdown(text string, w int) string {
	if w < 8 {
		w = 8
	}
	lines := strings.Split(text, "\n")
	var out strings.Builder
	inCode := false
	var codeLang string
	var codeBuf strings.Builder

	flush := func() {
		if codeBuf.Len() == 0 {
			inCode = false
			codeLang = ""
			return
		}
		out.WriteString(renderCodeBlock(codeBuf.String(), codeLang, w))
		out.WriteByte('\n')
		codeBuf.Reset()
		codeLang = ""
		inCode = false
	}

	for i, line := range lines {
		trimmed := strings.TrimRight(line, " \t\r")

		// fence toggle
		if isFence(trimmed) {
			if !inCode {
				inCode = true
				codeLang = fenceLang(trimmed)
				codeBuf.Reset()
				continue
			}
			flush()
			continue
		}
		if inCode {
			codeBuf.WriteString(trimmed)
			codeBuf.WriteByte('\n')
			continue
		}

		// blank line → paragraph break
		if strings.TrimSpace(trimmed) == "" {
			out.WriteByte('\n')
			continue
		}

		out.WriteString(renderMarkdownLine(trimmed, w))
		if i < len(lines)-1 {
			out.WriteByte('\n')
		}
	}
	if inCode {
		flush()
	}
	return strings.TrimRight(out.String(), "\n")
}

// renderMarkdownLine renders a single non-code source line (paragraph text,
// heading, list item, blockquote, hr) with inline formatting + wrapping.
func renderMarkdownLine(line string, w int) string {
	// hr
	if isHr(line) {
		return dimRule(w)
	}
	// heading
	if lvl, rest, ok := parseHeading(line); ok {
		txt := strings.TrimSpace(rest)
		st := accentStyle.Bold(true)
		if lvl >= 3 {
			st = mutedStyle.Bold(true)
		}
		return st.Render(strings.Repeat("#", min(lvl, 3)) + " " + txt)
	}
	// blockquote
	if strings.HasPrefix(strings.TrimSpace(line), ">") {
		inner := strings.TrimSpace(strings.TrimPrefix(strings.TrimSpace(line), ">"))
		inner = strings.TrimSpace(inner)
		segs := parseInline(inner)
		wrapped := wrapSegments(segs, w-2)
		var b strings.Builder
		for _, l := range strings.Split(wrapped, "\n") {
			b.WriteString(dimStyle.Render("▎ "))
			b.WriteString(thinkStyle.Render(l))
			b.WriteByte('\n')
		}
		return strings.TrimRight(b.String(), "\n")
	}
	// unordered list
	if pre, rest, ok := parseBullet(line); ok {
		return renderListItem("• ", pre, rest, w)
	}
	// ordered list
	if pre, rest, ok := parseOrdered(line); ok {
		return renderListItem(pre, pre, rest, w)
	}
	// paragraph
	segs := parseInline(line)
	return wrapSegments(segs, w)
}

// renderListItem renders a list bullet + wrapped continuation, hanging indent.
func renderListItem(marker, pre, rest string, w int) string {
	segs := parseInline(strings.TrimSpace(rest))
	indent := strings.Repeat(" ", lipgloss.Width(marker))
	wrapped := wrapSegments(segs, w-lipgloss.Width(marker))
	lines := strings.Split(wrapped, "\n")
	var b strings.Builder
	for i, l := range lines {
		if i == 0 {
			b.WriteString(mutedStyle.Render(marker))
		} else {
			b.WriteString(indent)
		}
		b.WriteString(l)
		if i < len(lines)-1 {
			b.WriteByte('\n')
		}
	}
	return b.String()
}

// renderCodeBlock renders a fenced block as a left-ruled panel. Code is never
// reflowed; over-long lines are truncated to fit.
func renderCodeBlock(code, lang string, w int) string {
	code = strings.TrimRight(code, "\n")
	if code == "" {
		code = ""
	}
	// layout: "│ " prefix ⇒ content width = w - 2 (full-width box at column 0)
	contentW := w - 2
	if contentW < 4 {
		contentW = 4
	}
	var b strings.Builder
	// top rule with optional lang label
	label := strings.TrimSpace(lang)
	if label == "" {
		label = "code"
	}
	top := "╭ " + label + " "
	fill := w - lipgloss.Width(top)
	if fill < 0 {
		fill = 0
	}
	b.WriteString(dimStyle.Render(top + strings.Repeat("─", fill)))
	b.WriteByte('\n')
	for _, raw := range strings.Split(code, "\n") {
		ln := raw
		if lipgloss.Width(ln) > contentW {
			ln = truncateRunes(ln, contentW)
		}
		b.WriteString(dimStyle.Render("│ "))
		b.WriteString(codeTextStyle.Render(ln))
		b.WriteByte('\n')
	}
	b.WriteString(dimStyle.Render("╰" + strings.Repeat("─", w-1)))
	return strings.TrimRight(b.String(), "\n")
}

// ---------------------------------------------------------------------------
// Inline parsing → styled segments
// ---------------------------------------------------------------------------

type segment struct {
	text  string
	style lipgloss.Style
}

func parseInline(s string) []segment {
	var segs []segment
	cur := segment{style: baseStyle}
	flush := func() {
		if cur.text != "" {
			segs = append(segs, cur)
		}
		cur = segment{style: baseStyle}
	}

	r := []rune(s)
	i := 0
	for i < len(r) {
		c := r[i]

		// inline code: `...`
		if c == '`' {
			end := indexRune(r, i+1, '`')
			if end > i {
				flush()
				segs = append(segs, segment{text: string(r[i+1 : end]), style: codeInlineStyle})
				i = end + 1
				continue
			}
		}

		// bold: **...** (check before italic)
		if c == '*' && i+1 < len(r) && r[i+1] == '*' {
			end := indexRune(r, i+2, '*')
			if end > i+1 && end+1 < len(r) && r[end+1] == '*' {
				flush()
				segs = append(segs, segment{text: string(r[i+2 : end]), style: boldBaseStyle})
				i = end + 2
				continue
			}
		}

		// italic: *...* or _..._
		if (c == '*' || c == '_') && !(i+1 < len(r) && r[i+1] == c) {
			end := indexRune(r, i+1, c)
			if end > i {
				flush()
				segs = append(segs, segment{text: string(r[i+1 : end]), style: italicStyle})
				i = end + 1
				continue
			}
		}

		// link: [text](url)
		if c == '[' {
			closeText := indexRune(r, i+1, ']')
			if closeText > i && closeText+1 < len(r) && r[closeText+1] == '(' {
				closeUrl := indexRune(r, closeText+2, ')')
				if closeUrl > closeText {
					flush()
					txt := string(r[i+1 : closeText])
					segs = append(segs, segment{text: txt, style: linkStyle})
					i = closeUrl + 1
					continue
				}
			}
		}

		cur.text += string(c)
		i++
	}
	flush()
	if segs == nil {
		segs = []segment{{text: "", style: baseStyle}}
	}
	return segs
}

// wrapSegments word-wraps styled segments to width w, preserving each word's
// style. Whitespace between words collapses to single spaces.
func wrapSegments(segs []segment, w int) string {
	if w < 1 {
		w = 1
	}
	type word struct {
		text  string
		style lipgloss.Style
	}
	var words []word
	for _, sg := range segs {
		for _, f := range strings.Fields(sg.text) {
			words = append(words, word{text: f, style: sg.style})
		}
	}
	if len(words) == 0 {
		return ""
	}
	var lines []string
	var cur strings.Builder
	curW := 0
	push := func() {
		lines = append(lines, cur.String())
		cur.Reset()
		curW = 0
	}
	for _, word := range words {
		ww := lipgloss.Width(word.text)
		// bare punctuation (trailing ".", ",", etc.) glues to the previous word
		// so "italic" + "." → "italic." rather than "italic ."
		glue := curW > 0 && isAllPunct(word.text)
		need := ww
		if curW > 0 && !glue {
			need++ // space
		}
		if curW+need > w && curW > 0 {
			push()
			cur.WriteString(word.style.Render(word.text))
			curW = ww
			continue
		}
		if curW > 0 && !glue {
			cur.WriteByte(' ')
			curW++
		}
		cur.WriteString(word.style.Render(word.text))
		curW += ww
	}
	if curW > 0 || len(lines) == 0 {
		push()
	}
	return strings.Join(lines, "\n")
}

// ---------------------------------------------------------------------------
// Small parsers
// ---------------------------------------------------------------------------

func isFence(line string) bool {
	t := strings.TrimSpace(line)
	return strings.HasPrefix(t, "```") || strings.HasPrefix(t, "~~~")
}

func fenceLang(line string) string {
	t := strings.TrimSpace(line)
	t = strings.TrimPrefix(t, "```")
	t = strings.TrimPrefix(t, "~~~")
	return strings.TrimSpace(t)
}

func isHr(line string) bool {
	t := strings.TrimSpace(line)
	if len(t) < 3 {
		return false
	}
	if !(strings.HasPrefix(t, "---") || strings.HasPrefix(t, "***") || strings.HasPrefix(t, "___")) {
		return false
	}
	for _, c := range t {
		if c != '-' && c != '*' && c != '_' && c != ' ' {
			return false
		}
	}
	return true
}

func parseHeading(line string) (level int, rest string, ok bool) {
	t := line
	for level = 0; level < len(t) && t[level] == '#'; level++ {
	}
	if level == 0 || level > 6 {
		return 0, "", false
	}
	if level >= len(t) || t[level] != ' ' {
		return 0, "", false
	}
	return level, t[level+1:], true
}

func parseBullet(line string) (prefix, rest string, ok bool) {
	t := strings.TrimLeft(line, " \t")
	if strings.HasPrefix(t, "- ") || strings.HasPrefix(t, "* ") || strings.HasPrefix(t, "+ ") {
		return "• ", t[2:], true
	}
	return "", "", false
}

func parseOrdered(line string) (marker, rest string, ok bool) {
	t := strings.TrimLeft(line, " \t")
	i := 0
	for i < len(t) && t[i] >= '0' && t[i] <= '9' {
		i++
	}
	if i == 0 || i+1 >= len(t) || t[i] != '.' || t[i+1] != ' ' {
		return "", "", false
	}
	return t[:i+2] + " ", t[i+2:], true
}

func indexRune(r []rune, from int, target rune) int {
	for i := from; i < len(r); i++ {
		if r[i] == target {
			return i
		}
	}
	return -1
}

// isAllPunct reports whether s is entirely punctuation (no letters/digits).
// Used to glue trailing "."/","/etc. to the preceding word so wrapping
// doesn't insert "italic ." instead of "italic.".
func isAllPunct(s string) bool {
	if s == "" {
		return false
	}
	for _, r := range s {
		if (r >= 'a' && r <= 'z') || (r >= 'A' && r <= 'Z') || (r >= '0' && r <= '9') {
			return false
		}
	}
	return true
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}
