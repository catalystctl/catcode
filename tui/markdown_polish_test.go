package main

import (
	"strings"
	"testing"
)

func TestTransformMermaidBlocks(t *testing.T) {
	in := "intro\n\n```mermaid\ngraph TD\n  A --> B\n  B --> C\n```\n\noutro"
	got := transformMermaidBlocks(in)
	if !strings.Contains(got, "Mermaid diagram · 3 lines") {
		t.Errorf("missing mermaid header line: %q", got)
	}
	if !strings.Contains(got, "```text") {
		t.Errorf("mermaid source should be wrapped in a ```text fence: %q", got)
	}
	if !strings.Contains(got, "A --> B") {
		t.Errorf("mermaid source must be preserved: %q", got)
	}
	if strings.Contains(got, "```mermaid") {
		t.Errorf("original mermaid fence should be gone: %q", got)
	}
	// Non-mermaid fences are untouched.
	in2 := "```rust\nfn main() {}\n```"
	if transformMermaidBlocks(in2) != in2 {
		t.Errorf("rust fence should be unchanged: %q", transformMermaidBlocks(in2))
	}
}

func TestTransformMermaidUnterminated(t *testing.T) {
	in := "```mermaid\ngraph TD\n  A --> B"
	got := transformMermaidBlocks(in)
	if !strings.Contains(got, "Mermaid diagram") {
		t.Errorf("unterminated mermaid should still produce header: %q", got)
	}
	if !strings.Contains(got, "A --> B") {
		t.Errorf("unterminated mermaid source should be preserved: %q", got)
	}
}

func TestPrettyMathGreekAndOps(t *testing.T) {
	cases := map[string]string{
		`\alpha + \beta`:     "α + β",
		`\sum_{i=1}^{n} x_i`: "∑ᵢ₌₁ⁿ xᵢ",
		`\int_0^\infty`:      "∫₀∞",
		`\sqrt{x}`:           "√x",
		`\sqrt{x^2 + y^2}`:   "√(x² + y²)",
		`\frac{a}{b}`:        "a⁄b",
		`x^2 + y^2 = r^2`:    "x² + y² = r²",
		`\pi \approx 3.14`:   "π ≈ 3.14",
		`\forall x \in S`:    "∀ x ∈ S",
	}
	for in, want := range cases {
		got := prettyMath(in)
		if got != want {
			t.Errorf("prettyMath(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestPrettyMathSuperscriptGroup(t *testing.T) {
	// x^{ab} → superscript group
	got := prettyMath(`x^{ab}`)
	want := "x" + toSuperscript("ab")
	if got != want {
		t.Errorf("prettyMath(x^{ab}) = %q, want %q", got, want)
	}
}

func TestPrettyMathUnknownCommandFallsBack(t *testing.T) {
	got := prettyMath(`\unknowncmd foo`)
	if !strings.Contains(got, "unknowncmd") {
		t.Errorf("unknown command should keep its name: %q", got)
	}
}

func TestTransformMathDisplay(t *testing.T) {
	in := "Pre\n\n$$\\sum_{i=1}^{n} x_i$$\n\nPost"
	got := transformMath(in)
	if !strings.Contains(got, "∑ᵢ₌₁ⁿ xᵢ") {
		t.Errorf("display math should be pretty-printed: %q", got)
	}
	if strings.Contains(got, "$$") {
		t.Errorf("$$ delimiters should be gone: %q", got)
	}
}

func TestTransformMathInline(t *testing.T) {
	in := "The area is $A = \\pi r^2$ for radius r."
	got := transformMath(in)
	if !strings.Contains(got, "π") || !strings.Contains(got, "r²") {
		t.Errorf("inline math should be pretty-printed: %q", got)
	}
	if strings.Contains(got, "$A = ") {
		t.Errorf("inline $ delimiters should be gone: %q", got)
	}
}

func TestTransformMathCurrencyUntouched(t *testing.T) {
	cases := []string{
		"Cost is $5 and $10.",
		"Revenue: $1,000,000 last year.",
		"Save $50 now!",
		"The \\$5 token and \\$6 fee.",
	}
	for _, in := range cases {
		got := transformMath(in)
		if got != in {
			t.Errorf("currency should be untouched: in=%q got=%q", in, got)
		}
	}
}

func TestTransformMathProtectsCode(t *testing.T) {
	// Inline code with $ should not be transformed.
	in := "Run `echo $HOME` now."
	got := transformMath(in)
	if !strings.Contains(got, "`echo $HOME`") {
		t.Errorf("inline code must be protected from math transform: %q", got)
	}
	// Fenced code with $ should not be transformed.
	in2 := "```bash\necho $PATH\n```"
	if transformMath(in2) != in2 {
		t.Errorf("fenced code must be protected: %q", transformMath(in2))
	}
}

func TestTransformMathMermaidSourceProtected(t *testing.T) {
	// After mermaid transform the source is in a ```text fence; math must not
	// touch $ inside it.
	in := "```mermaid\nA -- $5 --> B\n```"
	pre := preprocessMarkdownPolish(in)
	if !strings.Contains(pre, "$5") {
		t.Errorf("mermaid source $5 must survive math pass: %q", pre)
	}
}

func TestPostprocessHexSwatches(t *testing.T) {
	in := "Use color #ff0000 for errors."
	got := postprocessHexSwatches(in)
	if !strings.Contains(got, "#ff0000") {
		t.Errorf("hex code must be preserved: %q", got)
	}
	if !strings.Contains(got, "●") {
		t.Errorf("swatch dot must be appended: %q", got)
	}
	if !strings.Contains(got, "\x1b[38;2;255;0;0m") {
		t.Errorf("swatch must use truecolor red: %q", got)
	}
}

func TestPostprocessHexSwatchesShort(t *testing.T) {
	in := "Brand color #f00."
	got := postprocessHexSwatches(in)
	if !strings.Contains(got, "\x1b[38;2;255;0;0m●") {
		t.Errorf("3-digit hex #f00 -> red swatch: %q", got)
	}
}

func TestPostprocessHexSwatchesRespectsANSI(t *testing.T) {
	// An ANSI-colored span containing a hex code: the swatch must appear in
	// visible text and the active style must be restored after.
	in := "\x1b[31merror #00ff00 done\x1b[0m"
	got := postprocessHexSwatches(in)
	if !strings.Contains(got, "\x1b[38;2;0;255;0m●") {
		t.Errorf("green swatch must appear: %q", got)
	}
	// The original reset and red prefix must survive.
	if !strings.HasPrefix(got, "\x1b[31m") || !strings.HasSuffix(got, "\x1b[0m") {
		t.Errorf("ANSI framing must be preserved: %q", got)
	}
}

func TestPostprocessHexSwatchesNoHexUnchanged(t *testing.T) {
	in := "no colors here"
	if got := postprocessHexSwatches(in); got != in {
		t.Errorf("text without hex should be unchanged: %q", got)
	}
}

func TestPostprocessHexSwatchesNotInsideLongerHex(t *testing.T) {
	// #abcdef0 — 7 hex digits — should not match a 6-digit swatch (no boundary).
	in := "hash #abcdef0 trailing"
	got := postprocessHexSwatches(in)
	if strings.Contains(got, "●") {
		t.Errorf("hex embedded in longer hex run should not get a swatch: %q", got)
	}
}

func TestPreprocessMarkdownPolishIdempotentOnPlain(t *testing.T) {
	in := "Just a plain paragraph with no markdown or math."
	if got := preprocessMarkdownPolish(in); got != in {
		t.Errorf("plain text should pass through unchanged: %q", got)
	}
}
