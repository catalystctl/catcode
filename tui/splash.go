package main

import (
	"fmt"
	"math"
	"strings"
	"time"

	"charm.land/lipgloss/v2"
)

// Splash animation tuning. The busy-frame clock (~10 FPS) re-renders while the
// core is starting, so these time-based phases advance without a dedicated ticker.
const (
	splashOrbitCycle  = 1800 * time.Millisecond // orbit of the diamond sparks
	splashBarCycle    = 1400 * time.Millisecond // comet sweep across the loader bar
	splashBreathCycle = 2200 * time.Millisecond // soft amplitude / glow breath
	splashMarkCycle   = 900 * time.Millisecond  // central ◆ pulse
	// Minimum time the branded splash stays on screen. Local cores often emit
	// `ready` in <100ms — without a hold the animation never paints a frame.
	splashMinHold = 1800 * time.Millisecond
)

// splashHoldDoneMsg fires when the minimum splash hold elapses. gen ties it to
// a specific startCore so a restart's hold tick is ignored.
type splashHoldDoneMsg struct{ gen uint64 }

// splashMinHoldDuration returns how long to keep the splash after startCore.
// Reduced motion still gets a brief static brand beat so the boot is not a flash.
func (s *session) splashMinHoldDuration() time.Duration {
	if s.motionReduced() {
		return 400 * time.Millisecond
	}
	return splashMinHold
}

// showingSplash reports whether the branded startup panel should still paint.
// True while the core is starting, and for a short hold after a fast `ready` so
// the animation is actually visible. Conversation content always wins.
func (s *session) showingSplash() bool {
	if s.hasConversation() {
		return false
	}
	if s.coreStartGen == 0 || s.coreLifecycle == coreFailed {
		return false
	}
	if s.coreLifecycle == coreStarting {
		return true
	}
	// coreReady (or any post-start state): keep the brand up through min hold.
	hold := s.splashMinHoldDuration()
	if hold <= 0 || s.splashStartedAt.IsZero() {
		return false
	}
	return time.Since(s.splashStartedAt) < hold
}

// splashOrbitGlyphs are the rotating sparks that orbit the brand mark.
var splashOrbitGlyphs = []rune{'·', '˙', '˚', '✧', '✦', '✧', '˚', '˙'}

// splashBarRamp maps 0..1 intensity to a bar cell (same family as the working wave).
var splashBarRamp = []rune{' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'}

// splashPhase returns t∈[0,1) for a cycle length. Fixed at 0 when reduced motion.
func splashPhase(cycle time.Duration, reduced bool) float64 {
	if reduced || cycle <= 0 {
		return 0
	}
	return float64(time.Now().UnixNano()%int64(cycle)) / float64(int64(cycle))
}

// renderSplashScreen is the branded boot panel shown while the core is starting
// (and, briefly, before the first WindowSizeMsg). It keeps the "Starting" /
// "checking credentials" copy so existing startup tests still match.
func (s *session) renderSplashScreen(w, h int) string {
	if w < 1 {
		w = 1
	}
	if h < 1 {
		h = 1
	}
	reduced := s.motionReduced()
	compact := h < 12 || w < 36

	panelW := min(52, max(22, w-4))
	if compact {
		panelW = min(40, max(18, w-2))
	}
	// Bar sits inside the card padding; leave room for the rounded border + pad.
	barW := max(12, panelW-4)

	brand := s.renderSplashBrand(reduced, compact)
	bar := s.renderSplashBar(barW, reduced)
	status := accentStyle.Render("Starting…")
	detail := baseStyle.Render("Connecting to the core and checking credentials.")

	var rows []string
	if compact {
		rows = []string{brand, bar, status, dimStyle.Render("checking credentials")}
	} else {
		rows = []string{
			brand,
			"",
			bar,
			"",
			status,
			detail,
		}
		if coreVersion != "" && coreVersion != "dev" {
			rows = append(rows, "", dimStyle.Render("v"+coreVersion))
		} else if coreVersion == "dev" {
			rows = append(rows, "", dimStyle.Render("dev build"))
		}
	}

	inner := strings.Join(rows, "\n")
	var panel string
	if reduced {
		panel = surfacePanel(panelW).Render(inner)
	} else {
		panel = s.renderSplashCard(panelW, inner, barW)
	}
	return lipgloss.Place(w, h, lipgloss.Center, lipgloss.Center, panel)
}

// renderSplashBrand draws the diamond mark + wordmark, with orbiting sparks when
// motion is allowed. Compact terminals drop the orbit for a single pulsed mark.
func (s *session) renderSplashBrand(reduced, compact bool) string {
	mark := s.renderSplashMark(reduced)
	word := boldBaseStyle.Render("Catalyst") + dimStyle.Render(" Code")
	if compact || reduced {
		return mark + " " + word
	}
	// Independent top/bottom orbits (phase-inverted) — never reverse a styled
	// ANSI string; that mangles CSI sequences into garbage cells.
	top := s.renderSplashOrbit(false, 0)
	bot := s.renderSplashOrbit(false, 0.5)
	mid := mark + "  " + word
	return strings.Join([]string{top, mid, bot}, "\n")
}

// renderSplashMark pulses the brand diamond between dim and accent.
func (s *session) renderSplashMark(reduced bool) string {
	if reduced {
		return accentStyle.Render("◆")
	}
	phase := splashPhase(splashMarkCycle, false)
	// Ease: bright near 0 and 0.5 peaks feel like a twin spark.
	level := 0.45 + 0.55*math.Abs(math.Sin(2*math.Pi*phase))
	base := hexRGB(c.railDim)
	accent := hexRGB(c.accent)
	rgb := blendRGB(base, accent, level)
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(fmt.Sprintf("#%02x%02x%02x", rgb[0], rgb[1], rgb[2]))).
		Bold(true).
		Render("◆")
}

// renderSplashOrbit is a short row of sparks whose bright cell travels left→right.
// phaseOffset shifts the peak (0.5 ≈ opposite side) so top/bottom halos counter-rotate.
func (s *session) renderSplashOrbit(reduced bool, phaseOffset float64) string {
	const n = 11
	if reduced {
		return strings.Repeat("·", n)
	}
	phase := math.Mod(splashPhase(splashOrbitCycle, false)+phaseOffset, 1)
	breath := splashPhase(splashBreathCycle, false)
	amp := 0.65 + 0.35*math.Sin(2*math.Pi*breath)
	base := hexRGB(c.dim)
	accent := hexRGB(c.accent)
	var b strings.Builder
	for i := 0; i < n; i++ {
		pos := float64(i) / float64(n-1)
		d := math.Abs(pos - phase)
		if d > 0.5 {
			d = 1 - d
		}
		level := math.Max(0, 1-d*3) * amp
		gi := int(level * float64(len(splashOrbitGlyphs)-1))
		if gi < 0 {
			gi = 0
		}
		if gi >= len(splashOrbitGlyphs) {
			gi = len(splashOrbitGlyphs) - 1
		}
		rgb := blendRGB(base, accent, level)
		b.WriteString(lipgloss.NewStyle().
			Foreground(lipgloss.Color(fmt.Sprintf("#%02x%02x%02x", rgb[0], rgb[1], rgb[2]))).
			Render(string(splashOrbitGlyphs[gi])))
	}
	return b.String()
}

// renderSplashBar is a full-width (capped) comet loader under the brand.
func (s *session) renderSplashBar(width int, reduced bool) string {
	if width < 8 {
		width = 8
	}
	cells := make([]string, width)
	base := hexRGB(c.railDim)
	accent := hexRGB(c.accent)
	if reduced {
		// Static mid-fill so the bar still reads as a progress affordance.
		mid := width / 3
		for x := 0; x < width; x++ {
			level := 0.0
			if x < mid {
				level = 0.55
			}
			rgb := blendRGB(base, accent, level)
			ch := splashBarRamp[int(level*float64(len(splashBarRamp)-1))]
			cells[x] = lipgloss.NewStyle().
				Foreground(lipgloss.Color(fmt.Sprintf("#%02x%02x%02x", rgb[0], rgb[1], rgb[2]))).
				Render(string(ch))
		}
		return strings.Join(cells, "")
	}
	phase := splashPhase(splashBarCycle, false)
	breath := splashPhase(splashBreathCycle, false)
	amp := 0.75 + 0.25*math.Sin(2*math.Pi*breath)
	// Head position travels 0..1; a soft tail trails behind.
	for x := 0; x < width; x++ {
		pos := float64(x) / float64(width-1)
		// Distance behind the head (wrap so the comet loops cleanly).
		d := pos - phase
		if d > 0 {
			d -= 1
		}
		// d ∈ (-1, 0]; tail length ~0.35 of the bar.
		level := 0.0
		if d > -0.35 {
			level = (1 + d/0.35) * amp
		}
		// Soft leading glow just ahead of the head.
		ahead := pos - phase
		if ahead < 0 {
			ahead += 1
		}
		if ahead < 0.08 {
			level = math.Max(level, (1-ahead/0.08)*0.45*amp)
		}
		level = math.Min(math.Max(level, 0), 1)
		// Edge fade so the bar melts into the card padding.
		if edge := math.Min(float64(x), float64(width-1-x)) / 2; edge < 1 {
			level *= edge
		}
		ri := int(level*float64(len(splashBarRamp)-1) + 0.5)
		if ri < 0 {
			ri = 0
		}
		if ri >= len(splashBarRamp) {
			ri = len(splashBarRamp) - 1
		}
		rgb := blendRGB(base, accent, level)
		cells[x] = lipgloss.NewStyle().
			Foreground(lipgloss.Color(fmt.Sprintf("#%02x%02x%02x", rgb[0], rgb[1], rgb[2]))).
			Render(string(splashBarRamp[ri]))
	}
	return strings.Join(cells, "")
}

// renderSplashCard wraps splash body in a rounded accent card. Twin sparkline
// rules (reusing the comet bar) bookend the body so motion reads without
// fighting lipgloss border cell geometry each frame.
func (s *session) renderSplashCard(width int, inner string, barW int) string {
	rule := s.renderSplashBar(barW, false)
	body := rule + "\n" + inner + "\n" + rule
	return lipgloss.NewStyle().
		BorderStyle(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(c.accent)).
		BorderBackground(lipgloss.Color(c.surface)).
		Background(lipgloss.Color(c.surface)).
		Padding(0, 1).
		Width(width).
		Render(body)
}

// needsBusyFrames reports whether the ~10 FPS busy-frame clock should keep
// firing. Covers in-flight turns, pre-layout boot, and the startup splash
// (including the post-ready min-hold so the animation keeps advancing).
func (s *session) needsBusyFrames() bool {
	if s.busy || !s.ready {
		return true
	}
	return s.showingSplash()
}

// splashAnimatesInViewport reports whether the transcript viewport holds a
// time-varying splash that must be refresh()'d each busy frame (chrome-only
// animations like the working wave re-paint via View without SetContent).
func (s *session) splashAnimatesInViewport() bool {
	if s.motionReduced() {
		return false
	}
	return s.showingSplash()
}
