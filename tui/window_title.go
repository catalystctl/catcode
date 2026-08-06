package main

import (
	"os"
	"path/filepath"
	"strings"
	"time"
)

// titleSpinnerFrames is the window-title "working" animation. The busy-frame
// clock (~10 FPS) re-renders View while a turn runs, so a time-based frame
// index advances without a dedicated ticker (same pattern as the working wave).
var titleSpinnerFrames = []rune{'⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'}

// titleSpinnerInterval is how long each spinner frame holds. Close to the
// busy-frame interval so every re-render visibly advances the animation.
const titleSpinnerInterval = time.Second / 8

// projectName returns the current project name for the terminal window title:
// the basename of the directory catcode was launched in. Falls back to
// "catcode" when the directory cannot be determined.
func projectName() string {
	if wd, err := os.Getwd(); err == nil {
		if b := filepath.Base(wd); b != "" && b != "." && b != string(filepath.Separator) {
			return b
		}
	}
	return "catcode"
}

// sanitizeWindowTitle strips control characters (a newline/BEL/ESC inside an
// OSC 2 title would corrupt the escape sequence) and caps length.
func sanitizeWindowTitle(t string) string {
	var b strings.Builder
	n := 0
	for _, r := range t {
		if r < 0x20 || r == 0x7f {
			continue
		}
		if n >= 80 {
			break
		}
		b.WriteRune(r)
		n++
	}
	return b.String()
}

// windowTitle computes the terminal window title (set via View.WindowTitle;
// the Bubble Tea renderer writes OSC 2 whenever it changes and clears it on
// exit):
//   - while a turn runs (or the startup splash animates): spinner + name,
//     animating with the busy-frame clock (static glyph under reduced motion)
//   - while the agent needs attention (a turn just finished, an error fired,
//     or a blocking approval/ask/sudo/intercom prompt is open): a 🔔 prefix
//   - otherwise: the bare project name
func (s *session) windowTitle() string {
	name := s.projectName
	if name == "" {
		name = "catcode"
	}
	name = sanitizeWindowTitle(name)
	// Attention beats working: a blocking prompt pauses the busy-frame clock,
	// so a spinner would freeze anyway — and this is exactly the "needs
	// attention" case the bell exists for.
	if s.titleNeedsAttention() {
		return "🔔 " + name
	}
	if s.busy || s.showingSplash() || !s.ready {
		return string(s.titleBusyGlyph()) + " " + name
	}
	return name
}

// titleBusyGlyph picks the spinner frame for "now". Reduced motion pins one
// static glyph (same convention as the working wave / splash).
func (s *session) titleBusyGlyph() rune {
	if s.motionReduced() {
		return '◷'
	}
	i := time.Now().UnixNano() / int64(titleSpinnerInterval)
	return titleSpinnerFrames[int(i)%len(titleSpinnerFrames)]
}

// titleNeedsAttention reports whether the title should carry the bell: a
// blocking approval/ask/sudo/intercom prompt is open, the core failed, or a
// turn/error just finished (titleBell) and the user has not returned to the
// keyboard yet (handleKey / tea.FocusMsg clear titleBell).
func (s *session) titleNeedsAttention() bool {
	if s.pendingApproval != nil || s.pendingAsk != nil || s.pendingSudo != nil || s.pendingIntercom != nil {
		return true
	}
	if s.coreLifecycle == coreFailed {
		return true
	}
	return s.titleBell
}
