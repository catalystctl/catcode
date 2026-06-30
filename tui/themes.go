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
