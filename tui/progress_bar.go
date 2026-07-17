package main

import (
	"charm.land/bubbles/v2/progress"
	"charm.land/lipgloss/v2"
)

// staticProgressBar renders a non-animated bubbles/progress meter at the given
// ratio (0–1). Colors follow theme pressure thresholds when pressureTint is true.
func staticProgressBar(ratio float64, width int, full, empty rune, pressureTint bool) string {
	if width < 4 {
		width = 4
	}
	if ratio < 0 {
		ratio = 0
	}
	if ratio > 1 {
		ratio = 1
	}
	fullColor := lipgloss.Color(c.success)
	if pressureTint {
		switch {
		case ratio >= 0.9:
			fullColor = lipgloss.Color(c.err)
		case ratio >= 0.7:
			fullColor = lipgloss.Color(c.warn)
		}
	}
	m := progress.New(
		progress.WithWidth(width),
		progress.WithoutPercentage(),
		progress.WithFillCharacters(full, empty),
		progress.WithColors(fullColor),
	)
	m.EmptyColor = lipgloss.Color(c.dim)
	return m.ViewAs(ratio)
}

// renderUsageBar draws a filled/empty progress bar for /usage windows.
func renderUsageBar(ratio float64, width int) string {
	return staticProgressBar(ratio, width, '█', '░', true)
}
