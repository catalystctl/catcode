package main

import (
	"fmt"
	"math"
	"os"
	"strconv"
	"strings"

	"charm.land/lipgloss/v2"
)

// ---------------------------------------------------------------------------
// Themes
//
// The palette `c` and all derived style vars are package-level vars so they can
// be reassigned at runtime by setTheme. Each theme is a named colour set; the
// default (mocha) matches the original hardcoded Catppuccin Mocha values.
// ---------------------------------------------------------------------------

type theme struct {
	name    string
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
}

var themes = []theme{
	// --- Catalyst (Obsidian design system) — default ---
	// Matches the catalystctl frontend: pure neutral-gray with a
	// warm peach accent. Source: catalyst-website/src/styles/global.css.
	// Kept first so it is the default for new users; existing users keep their
	// saved theme (lookups are by name, not index).
	{
		name:    "catalyst",
		bg:      "#1a1a1a", // 0 0% 10% (gray, not near-black)
		fg:      "#f0f0f0", // 0 0% 94%
		dim:     "#4d4d4d", // subdued neutral used by the authored Catalyst palette
		muted:   "#858585", // 0 0% 52%
		accent:  "#cf8a59", // peach  25 55% 58%
		user:    "#cf8a59", // peach (terminal prompt)
		assist:  "#f0f0f0", // 0 0% 94%
		tool:    "#f59f0a", // amber  38 92% 50%
		success: "#3bde77", // green 142 71% 55%
		warn:    "#f59f0a", // amber  38 92% 50%
		err:     "#ef4343", // red    0 84% 60%
	},
	{
		name:    "mocha",
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
	},
	{
		name:    "latte",
		bg:      "#eff1f5",
		fg:      "#4c4f69",
		dim:     "#9ca0b0",
		muted:   "#6c6f85",
		accent:  "#209fb5",
		user:    "#1e66f5",
		assist:  "#4c4f69",
		tool:    "#df8e1d",
		success: "#40a02b",
		warn:    "#fe640b",
		err:     "#d20f39",
	},
	{
		name:    "tokyo",
		bg:      "#1a1b26",
		fg:      "#c0caf5",
		dim:     "#565f89",
		muted:   "#9aa5ce",
		accent:  "#7aa2f7",
		user:    "#bb9af7",
		assist:  "#c0caf5",
		tool:    "#e0af68",
		success: "#9ece6a",
		warn:    "#ff9e64",
		err:     "#f7768e",
	},
	{
		name:    "gruvbox",
		bg:      "#282828",
		fg:      "#ebdbb2",
		dim:     "#7c6f64",
		muted:   "#a89984",
		accent:  "#83a598",
		user:    "#458588",
		assist:  "#ebdbb2",
		tool:    "#fabd2f",
		success: "#b8bb26",
		warn:    "#fe8019",
		err:     "#fb4934",
	},
	// --- Catppuccin family (Frappé + Macchiato; Mocha/Latte already above) ---
	{
		name:    "frappe",
		bg:      "#303446",
		fg:      "#c6d0f5",
		dim:     "#737994",
		muted:   "#a5adce",
		accent:  "#85c1dc",
		user:    "#8caaee",
		assist:  "#c6d0f5",
		tool:    "#e5c890",
		success: "#a6d189",
		warn:    "#ef9f76",
		err:     "#e78284",
	},
	{
		name:    "macchiato",
		bg:      "#24273a",
		fg:      "#cad3f5",
		dim:     "#6e738d",
		muted:   "#a5adcb",
		accent:  "#7dc4e4",
		user:    "#8aadf4",
		assist:  "#cad3f5",
		tool:    "#eed49f",
		success: "#a6da95",
		warn:    "#f5a97f",
		err:     "#ed8796",
	},
	// --- Popular dark palettes ---
	{
		name:    "nord",
		bg:      "#2e3440",
		fg:      "#d8dee9",
		dim:     "#4c566a",
		muted:   "#81a1c1",
		accent:  "#88c0d0",
		user:    "#5e81ac",
		assist:  "#d8dee9",
		tool:    "#ebcb8b",
		success: "#a3be8c",
		warn:    "#d08770",
		err:     "#bf616a",
	},
	{
		name:    "dracula",
		bg:      "#282a36",
		fg:      "#f8f8f2",
		dim:     "#6272a4",
		muted:   "#abb2d6",
		accent:  "#8be9fd",
		user:    "#bd93f9",
		assist:  "#f8f8f2",
		tool:    "#f1fa8c",
		success: "#50fa7b",
		warn:    "#ffb86c",
		err:     "#ff5555",
	},
	{
		name:    "rose-pine",
		bg:      "#191724",
		fg:      "#e0def4",
		dim:     "#6e6a86",
		muted:   "#908caa",
		accent:  "#ebbcba",
		user:    "#c4a7e7",
		assist:  "#e0def4",
		tool:    "#f6c177",
		success: "#9ccfd8",
		warn:    "#f6a07a",
		err:     "#eb6f92",
	},
	{
		name:    "rose-pine-moon",
		bg:      "#232136",
		fg:      "#e0def4",
		dim:     "#6e6a86",
		muted:   "#908caa",
		accent:  "#ea9a97",
		user:    "#c4a7e7",
		assist:  "#e0def4",
		tool:    "#f6c177",
		success: "#9ccfd8",
		warn:    "#f6a07a",
		err:     "#eb6f92",
	},
	{
		name:    "kanagawa",
		bg:      "#1f1f28",
		fg:      "#dcd7ba",
		dim:     "#727169",
		muted:   "#c8c093",
		accent:  "#7e9cd8",
		user:    "#957fb8",
		assist:  "#dcd7ba",
		tool:    "#e6c384",
		success: "#98bb6c",
		warn:    "#ffa066",
		err:     "#e46a78",
	},
	{
		name:    "everforest",
		bg:      "#2d353b",
		fg:      "#d3c6aa",
		dim:     "#475558",
		muted:   "#939992",
		accent:  "#a7c080",
		user:    "#d699b6",
		assist:  "#d3c6aa",
		tool:    "#dbbc7f",
		success: "#83c092",
		warn:    "#e69875",
		err:     "#e67e80",
	},
	{
		name:    "monokai",
		bg:      "#2d2a2e",
		fg:      "#fcfcfa",
		dim:     "#727072",
		muted:   "#939293",
		accent:  "#78dce8",
		user:    "#ab9df2",
		assist:  "#fcfcfa",
		tool:    "#ffd866",
		success: "#a9dc76",
		warn:    "#fc9867",
		err:     "#ff6188",
	},
	{
		name:    "one-dark",
		bg:      "#282c34",
		fg:      "#abb2bf",
		dim:     "#5c6370",
		muted:   "#848b98",
		accent:  "#61afef",
		user:    "#c678dd",
		assist:  "#abb2bf",
		tool:    "#e5c07b",
		success: "#98c379",
		warn:    "#d19a66",
		err:     "#e06c75",
	},
	{
		name:    "solarized-dark",
		bg:      "#002b36",
		fg:      "#93a1a1",
		dim:     "#586e75",
		muted:   "#839496",
		accent:  "#268bd2",
		user:    "#6c71c4",
		assist:  "#93a1a1",
		tool:    "#b58900",
		success: "#859900",
		warn:    "#cb4b16",
		err:     "#dc322f",
	},
	{
		name:    "github-dark",
		bg:      "#0d1117",
		fg:      "#c9d1d9",
		dim:     "#6e7681",
		muted:   "#8b949e",
		accent:  "#58a6ff",
		user:    "#bc8cff",
		assist:  "#c9d1d9",
		tool:    "#d29922",
		success: "#3fb950",
		warn:    "#db6d28",
		err:     "#f85149",
	},
	{
		name:    "tokyo-storm",
		bg:      "#24283b",
		fg:      "#c0caf5",
		dim:     "#414868",
		muted:   "#565f89",
		accent:  "#7dcfff",
		user:    "#bb9af7",
		assist:  "#c0caf5",
		tool:    "#e0af68",
		success: "#9ece6a",
		warn:    "#ff9e64",
		err:     "#f7768e",
	},
	{
		name:    "ayu-dark",
		bg:      "#0b0e14",
		fg:      "#b3b1ad",
		dim:     "#3d424d",
		muted:   "#6c7380",
		accent:  "#ffb454",
		user:    "#59c2ff",
		assist:  "#b3b1ad",
		tool:    "#f0c674",
		success: "#aad94c",
		warn:    "#ff8f40",
		err:     "#d95757",
	},
	{
		name:    "synthwave",
		bg:      "#2a2139",
		fg:      "#f4eee4",
		dim:     "#4a3f6b",
		muted:   "#a8a0c8",
		accent:  "#ff7edb",
		user:    "#36f9f6",
		assist:  "#f4eee4",
		tool:    "#fede5d",
		success: "#72f1b8",
		warn:    "#ff8b39",
		err:     "#fe4450",
	},
	{
		name:    "matrix",
		bg:      "#0a0e0a",
		fg:      "#33ff66",
		dim:     "#1f3a1f",
		muted:   "#5a8f5a",
		accent:  "#00ffaa",
		user:    "#66ffcc",
		assist:  "#33ff66",
		tool:    "#ccff66",
		success: "#00ff66",
		warn:    "#ffd700",
		err:     "#ff4444",
	},
	{
		name:    "cobalt",
		bg:      "#0d1b2a",
		fg:      "#cbd5e1",
		dim:     "#2a3f5f",
		muted:   "#6b7fa0",
		accent:  "#4cc9f0",
		user:    "#b388eb",
		assist:  "#cbd5e1",
		tool:    "#f0c674",
		success: "#4ade80",
		warn:    "#ff9e64",
		err:     "#ef5a6f",
	},
	{
		name:    "midnight",
		bg:      "#0d0d10",
		fg:      "#c5c8c9",
		dim:     "#3a3a40",
		muted:   "#6a6a74",
		accent:  "#7c8aff",
		user:    "#9d7cff",
		assist:  "#c5c8c9",
		tool:    "#d4c08a",
		success: "#7cc98a",
		warn:    "#d49a5a",
		err:     "#d96a6a",
	},
	{
		name:    "high-contrast",
		bg:      "#000000",
		fg:      "#ffffff",
		dim:     "#808080",
		muted:   "#c0c0c0",
		accent:  "#00ffff",
		user:    "#ff00ff",
		assist:  "#ffffff",
		tool:    "#ffff00",
		success: "#00ff00",
		warn:    "#ff8000",
		err:     "#ff0000",
	},
	// --- Light themes ---
	{
		name:    "rose-pine-dawn",
		bg:      "#faf4ed",
		fg:      "#575279",
		dim:     "#bdae93",
		muted:   "#797593",
		accent:  "#d7827e",
		user:    "#286983",
		assist:  "#575279",
		tool:    "#ea9d34",
		success: "#56949f",
		warn:    "#d68553",
		err:     "#b4637a",
	},
	{
		name:    "one-light",
		bg:      "#fafafa",
		fg:      "#383a42",
		dim:     "#c8c8ce",
		muted:   "#a0a1a7",
		accent:  "#4078f2",
		user:    "#a626a4",
		assist:  "#383a42",
		tool:    "#c18401",
		success: "#50a14f",
		warn:    "#da6b3b",
		err:     "#e45649",
	},
	{
		name:    "solarized-light",
		bg:      "#fdf6e3",
		fg:      "#586e75",
		dim:     "#93a1a1",
		muted:   "#657b83",
		accent:  "#268bd2",
		user:    "#6c71c4",
		assist:  "#586e75",
		tool:    "#b58900",
		success: "#859900",
		warn:    "#cb4b16",
		err:     "#dc322f",
	},
	{
		name:    "github-light",
		bg:      "#ffffff",
		fg:      "#1f2328",
		dim:     "#6e7781",
		muted:   "#656d76",
		accent:  "#0969da",
		user:    "#8250df",
		assist:  "#1f2328",
		tool:    "#9a6700",
		success: "#1a7f37",
		warn:    "#bc4c00",
		err:     "#d1242f",
	},
	{
		name:    "tokyo-day",
		bg:      "#e1e2e7",
		fg:      "#373641",
		dim:     "#a1a6c0",
		muted:   "#6172b0",
		accent:  "#2e7de9",
		user:    "#9854b1",
		assist:  "#373641",
		tool:    "#8c6c3e",
		success: "#587539",
		warn:    "#b15c00",
		err:     "#f52a65",
	},
	{
		name:    "gruvbox-light",
		bg:      "#fbf1c7",
		fg:      "#3c3836",
		dim:     "#bdae93",
		muted:   "#928374",
		accent:  "#076678",
		user:    "#8f3f71",
		assist:  "#3c3836",
		tool:    "#b57614",
		success: "#79740e",
		warn:    "#af3a03",
		err:     "#9d0006",
	},
	{
		name:    "paper",
		bg:      "#f4ecd8",
		fg:      "#3e3a35",
		dim:     "#c9bfa6",
		muted:   "#8a7f6a",
		accent:  "#3d6e88",
		user:    "#7a4e8e",
		assist:  "#3e3a35",
		tool:    "#b07d3b",
		success: "#4a7c3e",
		warn:    "#c4761a",
		err:     "#b23a3a",
	},
}

var activeTheme = themes[0]

func init() { applyTheme(activeTheme) }

// setTheme looks up a theme by name (case-insensitive), applies it to the
// palette `c` and rebuilds every derived style var. Returns false if not found.
func setTheme(name string) bool {
	for _, t := range themes {
		if strings.EqualFold(t.name, name) {
			activeTheme = t
			applyTheme(t)
			return true
		}
	}
	return false
}

// themeIsDark reports whether the active theme background is dark (for Bubbles
// DefaultStyles(isDark) helpers).
func themeIsDark() bool {
	rgb := hexRGB(activeTheme.bg)
	// Relative luminance approximation; thresholds match typical dark UIs.
	lum := (0.2126*float64(rgb[0]) + 0.7152*float64(rgb[1]) + 0.0722*float64(rgb[2])) / 255
	return lum < 0.5
}

// applyTheme mutates the package palette, derives the structural tones, then
// rebuilds every style var to match a theme.
func applyTheme(t theme) {
	c.bg = t.bg
	c.fg = t.fg
	c.dim = t.dim
	c.muted = t.muted
	c.accent = t.accent
	c.user = t.user
	c.assist = t.assist
	c.tool = t.tool
	c.success = t.success
	c.warn = t.warn
	c.err = t.err
	// Theme authors choose the aesthetic palette; these two semantic colours
	// make that palette safe for small terminal text and structural boundaries.
	// They only move a colour toward the theme foreground when needed.
	c.secondary = ensureThemeContrast(t.muted, t.bg, t.fg, 4.5)
	c.decor = ensureThemeContrast(t.dim, t.bg, t.fg, 3.0)
	// Derive the surface/sunken/rail/soft tones from the authored palette so the
	// structural hierarchy stays consistent across light and dark themes while
	// the authored hex values (Catalyst included) are never touched.
	deriveSurfaceTones(t, themeIsDark())
	rebuildThemeStyles()
}

// rebuildThemeStyles assigns every package style var from the current palette.
// Single source for both the NO_COLOR and colour branches; called by applyTheme
// on every theme switch so nothing goes stale.
func rebuildThemeStyles() {
	if colorsDisabled() {
		baseStyle = lipgloss.NewStyle()
		boldBaseStyle = lipgloss.NewStyle().Bold(true)
		dimStyle = lipgloss.NewStyle()
		mutedStyle = lipgloss.NewStyle()
		accentStyle = lipgloss.NewStyle().Bold(true)
		successStyle = lipgloss.NewStyle().Bold(true)
		errStyle = lipgloss.NewStyle().Bold(true)
		warnStyle = lipgloss.NewStyle().Bold(true)
		assistantStyle = lipgloss.NewStyle()
		thinkStyle = lipgloss.NewStyle().Italic(true)
		toolNameStyle = lipgloss.NewStyle().Bold(true)
		toolDetailStyle = lipgloss.NewStyle()
		resultStyle = lipgloss.NewStyle()
		headerStyle = lipgloss.NewStyle().Bold(true)
		keyHintStyle = lipgloss.NewStyle()
		separatorStyle = lipgloss.NewStyle()
		inputPromptStyle = lipgloss.NewStyle().Bold(true)
		placeholderStyle = lipgloss.NewStyle()
		codeTextStyle = lipgloss.NewStyle()
		codeInlineStyle = lipgloss.NewStyle().Bold(true)
		italicStyle = lipgloss.NewStyle().Italic(true)
		linkStyle = lipgloss.NewStyle().Underline(true)
		roleUserStyle = lipgloss.NewStyle().Bold(true)
		roleAssistantStyle = lipgloss.NewStyle().Bold(true)
		roleThinkStyle = lipgloss.NewStyle().Italic(true)
		roleToolStyle = lipgloss.NewStyle().Bold(true)
		roleResultStyle = lipgloss.NewStyle().Bold(true)
		roleErrorStyle = lipgloss.NewStyle().Bold(true)
		roleWarnStyle = lipgloss.NewStyle().Bold(true)
		roleSuccessStyle = lipgloss.NewStyle().Bold(true)
		// Keep semantic colors on the style objects: diff classification and
		// third-party renderers inspect these values. Full View output is stripped
		// of ANSI below when NO_COLOR is active.
		toolDiffAdded = lipgloss.NewStyle().Foreground(lipgloss.Color(c.success))
		toolDiffRemoved = lipgloss.NewStyle().Foreground(lipgloss.Color(c.err))
		toolDiffContext = lipgloss.NewStyle().Foreground(lipgloss.Color(c.muted))
		toolDiffMeta = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent)).Bold(true)
		roToolNameStyle = lipgloss.NewStyle().Bold(true)
		errOutStyle = lipgloss.NewStyle().Bold(true)
		errRuleStyle = lipgloss.NewStyle().Bold(true)
		surfaceStyle = lipgloss.NewStyle()
		sunkenStyle = lipgloss.NewStyle()
		railStyle = lipgloss.NewStyle()
		userRailStyle = lipgloss.NewStyle().Bold(true)
		composerBorderStyle = lipgloss.NewStyle()
		cardStyle = lipgloss.NewStyle().BorderStyle(lipgloss.RoundedBorder()).Padding(0, 1)
		recessedStyle = lipgloss.NewStyle().BorderStyle(lipgloss.RoundedBorder()).Padding(0, 1)
		hairlineStyle = lipgloss.NewStyle()
		return
	}

	baseStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.fg))
	boldBaseStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.fg)).Bold(true)
	dimStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.secondary))
	mutedStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.secondary))
	accentStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent)).Bold(true)
	successStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.success))
	errStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.err))
	warnStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.warn)).Bold(true)
	assistantStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.assist))
	thinkStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.secondary)).Italic(true)
	toolNameStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.tool)).Bold(true)
	toolDetailStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.tool))
	resultStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.muted))
	headerStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent)).Bold(true)
	keyHintStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.secondary))
	separatorStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.decor))
	inputPromptStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent))
	placeholderStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.secondary))
	codeTextStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.fg))
	codeInlineStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.inlineCode))
	italicStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.fg)).Italic(true)
	linkStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent)).Underline(true)
	roleUserStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.user)).Bold(true)
	roleAssistantStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent)).Bold(true)
	roleThinkStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.secondary)).Italic(true)
	roleToolStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.tool)).Bold(true)
	roleResultStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.success)).Bold(true)
	roleErrorStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.err)).Bold(true)
	roleWarnStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.warn)).Bold(true)
	roleSuccessStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.success)).Bold(true)
	toolDiffAdded = lipgloss.NewStyle().Foreground(lipgloss.Color(c.success))
	toolDiffRemoved = lipgloss.NewStyle().Foreground(lipgloss.Color(c.err))
	toolDiffContext = lipgloss.NewStyle().Foreground(lipgloss.Color(c.muted))
	toolDiffMeta = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent))
	roToolNameStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent)).Bold(true)
	errOutStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.err))
	errRuleStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.err))
	// Structural surface / rail styles.
	surfaceStyle = lipgloss.NewStyle().Background(lipgloss.Color(c.surface))
	sunkenStyle = lipgloss.NewStyle().Background(lipgloss.Color(c.sunken))
	railStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.railDim))
	userRailStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent))
	composerBorderStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.railDim))
	// Redesign primitives. Border-only: no solid fill, so the terminal
	// background shows through after the text instead of a grey slab.
	cardStyle = lipgloss.NewStyle().
		BorderStyle(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(c.railDim)).
		Padding(0, 1)
	recessedStyle = lipgloss.NewStyle().
		BorderStyle(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(c.railDim)).
		Padding(0, 1)
	hairlineStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.railDim))
}

func parseHexColor(s string) ([3]float64, bool) {
	var rgb [3]float64
	if len(s) != 7 || s[0] != '#' {
		return rgb, false
	}
	for i := 0; i < 3; i++ {
		v, err := strconv.ParseUint(s[1+i*2:3+i*2], 16, 8)
		if err != nil {
			return rgb, false
		}
		rgb[i] = float64(v) / 255
	}
	return rgb, true
}

func relativeLuminance(rgb [3]float64) float64 {
	for i, v := range rgb {
		if v <= 0.04045 {
			rgb[i] = v / 12.92
		} else {
			rgb[i] = math.Pow((v+0.055)/1.055, 2.4)
		}
	}
	return 0.2126*rgb[0] + 0.7152*rgb[1] + 0.0722*rgb[2]
}

func colorContrast(a, b string) float64 {
	ar, aok := parseHexColor(a)
	br, bok := parseHexColor(b)
	if !aok || !bok {
		return 1
	}
	la, lb := relativeLuminance(ar), relativeLuminance(br)
	if la < lb {
		la, lb = lb, la
	}
	return (la + 0.05) / (lb + 0.05)
}

func ensureThemeContrast(color, bg, toward string, minimum float64) string {
	if colorContrast(color, bg) >= minimum {
		return color
	}
	from, ok1 := parseHexColor(color)
	to, ok2 := parseHexColor(toward)
	if !ok1 || !ok2 {
		return toward
	}
	best := toward
	for step := 1; step <= 100; step++ {
		t := float64(step) / 100
		candidate := fmt.Sprintf("#%02x%02x%02x",
			int(math.Round((from[0]+(to[0]-from[0])*t)*255)),
			int(math.Round((from[1]+(to[1]-from[1])*t)*255)),
			int(math.Round((from[2]+(to[2]-from[2])*t)*255)))
		best = candidate
		if colorContrast(candidate, bg) >= minimum {
			return candidate
		}
	}
	return best
}

func colorsDisabled() bool {
	_, disabled := os.LookupEnv("NO_COLOR")
	return disabled
}

func themeNames() []string {
	out := make([]string, len(themes))
	for i, t := range themes {
		out[i] = t.name
	}
	return out
}
