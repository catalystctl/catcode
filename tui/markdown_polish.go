package main

import (
	"fmt"
	"regexp"
	"strconv"
	"strings"
)

// ---------------------------------------------------------------------------
// Markdown polish — pre/post passes around the Glamour renderer.
//
// Three zero-dependency touches that match Grok Build's TUI polish surface:
//
//  1. Mermaid fenced blocks (```mermaid) are rewritten into a distinct, styled
//     block (a labeled blockquote header + the source as a ```text code block)
//     so diagrams stand out and stay copyable. Full image rendering is a
//     future, dependency-bearing step; this keeps the source visible and
//     clearly framed today.
//  2. LaTeX-style math ($$...$$ display, $...$ inline) is pretty-printed to
//     Unicode (Greek letters, super/subscripts, ∑ ∫ √ √ fractions, operators)
//     instead of rendering raw `$...$` literally.
//  3. Hex color codes (#rrggbb / #rgb) in rendered output get a truecolor ●
//     swatch appended after them.
//
// All passes are dependency-free and safe to no-op: if anything looks off,
// the worst case is the original text is preserved.
// ---------------------------------------------------------------------------

// preprocessMarkdownPolish runs BEFORE Glamour and rewrites mermaid fences +
// LaTeX math into markdown/Unicode Glamour renders well.
func preprocessMarkdownPolish(text string) string {
	text = transformMermaidBlocks(text)
	text = transformMath(text)
	return text
}

// postprocessMarkdownPolish runs AFTER Glamour/Legacy and decorates rendered
// ANSI output (hex color swatches).
func postprocessMarkdownPolish(text string) string {
	return postprocessHexSwatches(text)
}

// ---- Mermaid ---------------------------------------------------------------

func isMermaidFence(trimmed string) bool {
	if !strings.HasPrefix(trimmed, "```") {
		return false
	}
	info := strings.TrimSpace(strings.TrimPrefix(trimmed, "```"))
	if info == "" {
		return false
	}
	lang := info
	if idx := strings.IndexAny(info, " \t"); idx >= 0 {
		lang = info[:idx]
	}
	return strings.EqualFold(lang, "mermaid")
}

// transformMermaidBlocks rewrites ```mermaid fences into a labeled blockquote
// header + a ```text code block holding the source.
func transformMermaidBlocks(text string) string {
	lines := strings.Split(text, "\n")
	var out []string
	i := 0
	for i < len(lines) {
		trimmed := strings.TrimSpace(lines[i])
		if isMermaidFence(trimmed) {
			i++
			var body []string
			for i < len(lines) && !strings.HasPrefix(strings.TrimSpace(lines[i]), "```") {
				body = append(body, lines[i])
				i++
			}
			header := fmt.Sprintf("> ▒ Mermaid diagram · %d line%s", len(body), pluralS(len(body)))
			out = append(out, header, "", "```text")
			out = append(out, body...)
			out = append(out, "```")
			if i < len(lines) { // consume closing fence
				i++
			}
		} else {
			out = append(out, lines[i])
			i++
		}
	}
	return strings.Join(out, "\n")
}

// ---- Math ------------------------------------------------------------------

var (
	fenceRe       = regexp.MustCompile("(?s)`{3}[\\s\\S]*?`{3}")
	inlineCodeRe  = regexp.MustCompile("`[^`\n]+`")
	displayMathRe = regexp.MustCompile(`(?s)\$\$\s*([\s\S]+?)\s*\$\$`)
	inlineMathRe  = regexp.MustCompile(`\$([^\$\n]+?)\$`)
	// A strong math marker filter: a LaTeX command (\letter) or super/subscript.
	mathMarkerRe = regexp.MustCompile(`\\[a-zA-Z]|\^|_`)
)

// transformMath pretty-prints LaTeX math to Unicode, protecting fenced and
// inline code from transformation.
func transformMath(text string) string {
	// Protect fenced code blocks.
	var fences []string
	text = fenceRe.ReplaceAllStringFunc(text, func(m string) string {
		idx := len(fences)
		fences = append(fences, m)
		return fmt.Sprintf("\x00FENCE%d\x00", idx)
	})
	// Protect inline code spans.
	var codes []string
	text = inlineCodeRe.ReplaceAllStringFunc(text, func(m string) string {
		idx := len(codes)
		codes = append(codes, m)
		return fmt.Sprintf("\x00CODE%d\x00", idx)
	})

	// Display math first (multiline).
	text = displayMathRe.ReplaceAllStringFunc(text, func(m string) string {
		sub := displayMathRe.FindStringSubmatch(m)
		return "\n\n" + prettyMath(strings.TrimSpace(sub[1])) + "\n\n"
	})
	// Inline math — only when it has an unambiguous math marker (avoids $5).
	text = inlineMathRe.ReplaceAllStringFunc(text, func(m string) string {
		sub := inlineMathRe.FindStringSubmatch(m)
		inner := sub[1]
		if len(inner) > 200 || !mathMarkerRe.MatchString(inner) {
			return m
		}
		// No spaces flush against the delimiters (rules out "$ 5 off").
		if strings.HasPrefix(inner, " ") || strings.HasSuffix(inner, " ") {
			return m
		}
		return prettyMath(inner)
	})

	// Restore inline code then fences.
	text = restorePlaceholders(text, `\x00CODE(\d+)\x00`, codes)
	text = restorePlaceholders(text, `\x00FENCE(\d+)\x00`, fences)
	return text
}

func restorePlaceholders(text, pattern string, saved []string) string {
	re := regexp.MustCompile(pattern)
	return re.ReplaceAllStringFunc(text, func(m string) string {
		sub := re.FindStringSubmatch(m)
		idx, _ := strconv.Atoi(sub[1])
		if idx >= 0 && idx < len(saved) {
			return saved[idx]
		}
		return m
	})
}

// prettyMath converts a LaTeX math fragment to a Unicode pretty-printed
// string. Best-effort: unknown tokens are passed through readably.
func prettyMath(s string) string {
	var b strings.Builder
	r := []rune(s)
	i := 0
	for i < len(r) {
		c := r[i]
		switch c {
		case '\\':
			i = mathCommand(&b, r, i)
		case '^':
			i = writeScript(&b, r, i+1, true)
		case '_':
			i = writeScript(&b, r, i+1, false)
		case '{', '}':
			i++ // drop grouping braces (args already consumed by commands)
		default:
			b.WriteRune(c)
			i++
		}
	}
	return b.String()
}

// mathCommand handles a `\...` sequence starting at r[i] == '\\'.
func mathCommand(b *strings.Builder, r []rune, i int) int {
	// Read command name (letters after the backslash).
	j := i + 1
	for j < len(r) && isASCIILetter(r[j]) {
		j++
	}
	name := string(r[i+1 : j])

	// Control symbol (no letters): spacing commands collapse; literals pass through.
	if name == "" {
		if j >= len(r) {
			return j
		}
		switch r[j] {
		case ',', ':', ';', '!':
			return j + 1 // spacing — collapse to nothing
		case ' ', '\t':
			b.WriteRune(' ')
			return j + 1
		}
		b.WriteRune(r[j])
		return j + 1
	}

	if sym, ok := greekLetters[name]; ok {
		b.WriteString(sym)
		return j
	}
	if sym, ok := mathOps[name]; ok {
		b.WriteString(sym)
		return j
	}
	switch name {
	case "frac":
		a, ni := readGroup(r, j)
		c2, nj := readGroup(r, ni)
		b.WriteString(prettyMath(a))
		b.WriteString("⁄")
		b.WriteString(prettyMath(c2))
		return nj
	case "sqrt":
		k := j
		if k < len(r) && r[k] == '[' { // optional [n] root
			end := k + 1
			for end < len(r) && r[end] != ']' {
				end++
			}
			b.WriteString(toSuperscript(string(r[k+1 : end])))
			k = end + 1
		}
		body, ni := readGroup(r, k)
		b.WriteString("√")
		if len([]rune(body)) > 1 {
			b.WriteString("(")
			b.WriteString(prettyMath(body))
			b.WriteString(")")
		} else {
			b.WriteString(prettyMath(body))
		}
		return ni
	case "mathrm", "text", "mathbf", "mathit", "mathcal", "operatorname":
		body, ni := readGroup(r, j)
		b.WriteString(body)
		return ni
	case "mathbb":
		body, ni := readGroup(r, j)
		if sym, ok := blackboard[body]; ok {
			b.WriteString(sym)
		} else {
			b.WriteString(body)
		}
		return ni
	case "left", "right":
		return j // drop; keep the following delimiter char
	case "quad":
		b.WriteString("  ")
		return j
	case "qquad":
		b.WriteString("    ")
		return j
	}
	// Unknown control word: drop the backslash, keep the name readable.
	b.WriteString(name)
	return j
}

// writeScript converts a super/subscript token (single char or {...}) to
// Unicode and writes it. Returns the next index.
func writeScript(b *strings.Builder, r []rune, i int, isSuper bool) int {
	var tok string
	ni := i
	if i < len(r) && r[i] == '{' {
		body, next := readGroup(r, i)
		tok = body
		ni = next
	} else if i < len(r) && r[i] == '\\' {
		// Read a \\command as a unit so ^\\infty -> infinity, not superscript of bare \\.
		j := i + 1
		for j < len(r) && isASCIILetter(r[j]) {
			j++
		}
		tok = string(r[i:j])
		ni = j
	} else if i < len(r) {
		tok = string(r[i])
		ni = i + 1
	}
	tok = prettyMath(tok)
	if isSuper {
		b.WriteString(toSuperscript(tok))
	} else {
		b.WriteString(toSubscript(tok))
	}
	return ni
}

// readGroup reads a {...} group (returning inner text + index after) or a
// single char at r[i].
func readGroup(r []rune, i int) (string, int) {
	if i >= len(r) {
		return "", i
	}
	if r[i] == '{' {
		depth := 1
		j := i + 1
		for j < len(r) && depth > 0 {
			if r[j] == '{' {
				depth++
			} else if r[j] == '}' {
				depth--
			}
			if depth > 0 {
				j++
			}
		}
		return string(r[i+1 : j]), j + 1
	}
	return string(r[i]), i + 1
}

func isASCIILetter(c rune) bool {
	return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
}

// ---- Hex color swatches ----------------------------------------------------

func isHexRune(c rune) bool {
	return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F')
}

// tryHex returns the digit length (3 or 6) and RGB of a #hex color at r[i],
// or 0 if none. Requires a non-hex char after (word boundary).
func tryHex(r []rune, i int) (int, int, int, int) {
	n := len(r)
	start := i + 1
	// 6-digit first (superset of 3).
	if start+6 <= n {
		ok := true
		for k := 0; k < 6; k++ {
			if !isHexRune(r[start+k]) {
				ok = false
				break
			}
		}
		if ok && (start+6 == n || !isHexRune(r[start+6])) {
			rr := hexVal(r[start : start+2])
			gg := hexVal(r[start+2 : start+4])
			bb := hexVal(r[start+4 : start+6])
			return 6, rr, gg, bb
		}
	}
	// 3-digit.
	if start+3 <= n {
		ok := true
		for k := 0; k < 3; k++ {
			if !isHexRune(r[start+k]) {
				ok = false
				break
			}
		}
		if ok && (start+3 == n || !isHexRune(r[start+3])) {
			rr := hexVal([]rune{r[start], r[start]})
			gg := hexVal([]rune{r[start+1], r[start+1]})
			bb := hexVal([]rune{r[start+2], r[start+2]})
			return 3, rr, gg, bb
		}
	}
	return 0, 0, 0, 0
}

func hexVal(rr []rune) int {
	v, _ := strconv.ParseInt(string(rr), 16, 0)
	return int(v)
}

// postprocessHexSwatches appends a truecolor ● after #rrggbb / #rgb codes in
// visible (non-ANSI) text, restoring the active style afterward.
func postprocessHexSwatches(text string) string {
	var b strings.Builder
	r := []rune(text)
	n := len(r)
	lastSGR := ""
	i := 0
	for i < n {
		c := r[i]
		// Skip ANSI CSI sequences, tracking the active SGR.
		if c == 0x1b && i+1 < n && r[i+1] == '[' {
			j := i + 2
			for j < n && !(r[j] >= 0x40 && r[j] <= 0x7e) {
				j++
			}
			if j < n {
				j++ // include final byte
			}
			seq := string(r[i:j])
			b.WriteString(seq)
			if len(seq) > 0 && seq[len(seq)-1] == 'm' {
				lastSGR = seq
			}
			i = j
			continue
		}
		if c == '#' {
			ln, rr, gg, bb := tryHex(r, i)
			if ln > 0 {
				b.WriteString(string(r[i : i+1+ln]))
				fmt.Fprintf(&b, "\x1b[38;2;%d;%d;%dm●%s", rr, gg, bb, lastSGR)
				i += 1 + ln
				continue
			}
		}
		b.WriteRune(c)
		i++
	}
	return b.String()
}
