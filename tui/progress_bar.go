package main

import (
	"image/color"
	"sync"

	"charm.land/bubbles/v2/progress"
	"charm.land/lipgloss/v2"
)

// Reused progress models so footer/usage paints don't allocate a new spring
// every frame. Width and glyphs are set per call before ViewAs.
var (
	progressMu   sync.Mutex
	usageBar     progress.Model
	ctxBar       progress.Model
	progressInit sync.Once
)

func initProgressModels() {
	progressInit.Do(rebuildProgressModels)
}

func rebuildProgressModels() {
	usageBar = progress.New(
		progress.WithoutPercentage(),
		progress.WithFillCharacters('█', '░'),
		progress.WithColorFunc(pressureColorFunc),
	)
	usageBar.EmptyColor = lipgloss.Color(c.dim)
	ctxBar = progress.New(
		progress.WithoutPercentage(),
		progress.WithFillCharacters('▰', '▱'),
		progress.WithColorFunc(pressureColorFunc),
	)
	ctxBar.EmptyColor = lipgloss.Color(c.dim)
}

func pressureColorFunc(total, _ float64) color.Color {
	switch {
	case total >= 0.85:
		return lipgloss.Color(c.err)
	case total >= 0.6:
		return lipgloss.Color(c.warn)
	default:
		return lipgloss.Color(c.success)
	}
}

// staticProgressBar renders a non-animated bubbles/progress meter at ratio 0–1.
func staticProgressBar(ratio float64, width int, full, empty rune) string {
	initProgressModels()
	if width < 4 {
		width = 4
	}
	if ratio < 0 {
		ratio = 0
	}
	if ratio > 1 {
		ratio = 1
	}
	progressMu.Lock()
	defer progressMu.Unlock()
	m := &usageBar
	if full == '▰' {
		m = &ctxBar
	}
	m.Full = full
	m.Empty = empty
	m.SetWidth(width)
	m.EmptyColor = lipgloss.Color(c.dim)
	return m.ViewAs(ratio)
}

// newUsageWindowBar returns a spring-animated progress meter for /usage windows.
func newUsageWindowBar() progress.Model {
	bar := progress.New(
		progress.WithoutPercentage(),
		progress.WithFillCharacters('█', '░'),
		progress.WithColorFunc(pressureColorFunc),
	)
	bar.EmptyColor = lipgloss.Color(c.dim)
	return bar
}

// renderUsageBar draws a filled/empty progress bar for /usage windows.
func renderUsageBar(ratio float64, width int) string {
	return staticProgressBar(ratio, width, '█', '░')
}

// renderContextBar draws the compact footer context-window meter.
func renderContextBar(ratio float64, width int) string {
	return staticProgressBar(ratio, width, '▰', '▱')
}

// refreshProgressTheme rebuilds meters after a theme switch.
func refreshProgressTheme() {
	progressMu.Lock()
	defer progressMu.Unlock()
	rebuildProgressModels()
}
