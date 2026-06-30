package main

import (
	"strings"
	"testing"

	"github.com/charmbracelet/lipgloss"
)

// TestDiffLineStyle checks that each unified-diff marker maps to the expected
// theme style (by foreground color, which is environment-independent) and
// that added/removed lines render with distinct colors.
func TestDiffLineStyle(t *testing.T) {
	cases := []struct {
		line string
		want lipgloss.Style
	}{
		{"+added line", toolDiffAdded},
		{"-removed line", toolDiffRemoved},
		{" context line", toolDiffContext},
		{"plain context", toolDiffContext},
		{"@@ -1,3 +1,4 @@", toolDiffMeta},
		{"--- a/file.go", toolDiffMeta},
		{"+++ b/file.go", toolDiffMeta},
		{"diff --git a/file.go b/file.go", toolDiffMeta},
		{"\\ No newline at end of file", toolDiffMeta},
	}
	for _, c := range cases {
		got := diffLineStyle(c.line).GetForeground()
		want := c.want.GetForeground()
		if got != want {
			t.Errorf("diffLineStyle(%q) foreground = %v, want %v", c.line, got, want)
		}
	}
	// "+++ " / "--- " must classify as meta, NOT as added/removed (the trap: a
	// file header line starts with + / -).
	if diffLineStyle("+++ b/x.go").GetForeground() != toolDiffMeta.GetForeground() {
		t.Error("\"+++ \" line should classify as meta, not added")
	}
	if diffLineStyle("--- a/x.go").GetForeground() != toolDiffMeta.GetForeground() {
		t.Error("\"--- \" line should classify as meta, not removed")
	}
	// added and removed must be styled distinctly (different colors).
	if diffLineStyle("+x").GetForeground() == diffLineStyle("-y").GetForeground() {
		t.Error("added and removed lines should use distinct foreground colors")
	}
}

// TestRenderDiffPanel exercises the rendered panel: line classification shows
// up as the right text, truncation/expand hints appear, and small widths don't
// break it.
func TestRenderDiffPanel(t *testing.T) {
	const diff = `--- a/main.go
+++ b/main.go
@@ -1,3 +1,4 @@
 func main() {
-oldLine()
+newLine()
+extra()
 }`

	// expanded: every raw line present (stripped of ANSI) + the header path.
	out := stripANSI(renderDiffPanel(diff, true, 80))
	for _, want := range []string{
		"main.go", "@@ -1,3 +1,4 @@", "-oldLine()", "+newLine()", "+extra()", "func main() {",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("expanded diff missing %q:\n%s", want, out)
		}
	}
	if !strings.Contains(out, "ctrl+o collapse") {
		t.Errorf("expanded long diff missing collapse hint:\n%s", out)
	}

	// truncation: not expanded + >3 raw lines => a ctrl+o expand hint, the right
	// hidden-line count, and later lines suppressed.
	trunc := stripANSI(renderDiffPanel(diff, false, 80))
	if !strings.Contains(trunc, "ctrl+o expand") {
		t.Errorf("truncated diff missing expand hint:\n%s", trunc)
	}
	// 8 raw lines - 3 shown = 5 hidden.
	if !strings.Contains(trunc, "+5 line(s)") {
		t.Errorf("truncated diff missing hidden-line count:\n%s", trunc)
	}
	if strings.Contains(trunc, "+extra()") || strings.Contains(trunc, "+newLine()") {
		t.Errorf("truncated diff should hide later lines, got:\n%s", trunc)
	}

	// robustness to a small width: must not panic and still surface the header.
	narrow := stripANSI(renderDiffPanel(diff, true, 10))
	if !strings.Contains(narrow, "main.go") {
		t.Errorf("narrow diff missing header:\n%s", narrow)
	}

	// empty / blank diffs render an empty panel.
	if got := renderDiffPanel("", true, 80); got != "" {
		t.Errorf("empty diff should render empty, got %q", got)
	}
	if got := renderDiffPanel("   \n  ", true, 80); got != "" {
		t.Errorf("blank diff should render empty, got %q", got)
	}

	// a short diff (<= headLines) shows no hint either way.
	const short = `@@ -1,1 +1,1 @@
-old()
+new()`
	if strings.Contains(stripANSI(renderDiffPanel(short, false, 80)), "ctrl+o") {
		t.Error("short diff should not show a truncation hint")
	}
}
