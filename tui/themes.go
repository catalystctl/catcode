package main

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
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

// applyTheme mutates the package palette + style vars to match a theme.
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

	baseStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.fg))
	boldBaseStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.fg)).Bold(true)
	dimStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.dim))
	mutedStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.muted))
	accentStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent)).Bold(true)
	successStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.success))
	errStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.err))
	warnStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.warn)).Bold(true)
	assistantStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.assist))
	thinkStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.dim)).Italic(true)
	toolNameStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.tool)).Bold(true)
	toolDetailStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.tool))
	resultStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.muted))
	headerStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent)).Bold(true)
	keyHintStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.dim))
	separatorStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.dim))
	inputPromptStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent))
	placeholderStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.dim))
	codeTextStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.fg))
	codeInlineStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.tool))
	italicStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.fg)).Italic(true)
	linkStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent)).Underline(true)
	roleUserStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.user)).Bold(true)
	roleAssistantStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent)).Bold(true)
	roleThinkStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.dim)).Italic(true)
	roleToolStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.tool)).Bold(true)
	roleResultStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.success)).Bold(true)
	roleErrorStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.err)).Bold(true)
	roleWarnStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.warn)).Bold(true)
	roleSuccessStyle = lipgloss.NewStyle().Foreground(lipgloss.Color(c.success)).Bold(true)
	toolDiffAdded = lipgloss.NewStyle().Foreground(lipgloss.Color(c.success))
	toolDiffRemoved = lipgloss.NewStyle().Foreground(lipgloss.Color(c.err))
	toolDiffContext = lipgloss.NewStyle().Foreground(lipgloss.Color(c.muted))
	toolDiffMeta = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent))
}

func themeNames() []string {
	out := make([]string, len(themes))
	for i, t := range themes {
		out[i] = t.name
	}
	return out
}
