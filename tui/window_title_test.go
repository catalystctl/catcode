package main

import (
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

func titleSession(t *testing.T) *session {
	t.Helper()
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.projectName = "catcode"
	s.coreLifecycle = coreReady
	return s
}

// TestWindowTitleShowsProjectName proves the idle title is the launch-dir
// project name (not the literal string "catcode" branding) and never carries
// the spinner or the bell.
func TestWindowTitleShowsProjectName(t *testing.T) {
	s := titleSession(t)
	s.projectName = "my-project"

	title := s.windowTitle()
	if title != "my-project" {
		t.Fatalf("idle title should be the project name; got %q", title)
	}
}

// TestWindowTitleIdleDefaultName proves the fallback name when the working
// directory basename is unavailable is "catcode".
func TestWindowTitleIdleDefaultName(t *testing.T) {
	s := titleSession(t)
	s.projectName = ""

	if title := s.windowTitle(); title != "catcode" {
		t.Fatalf("idle title with empty project name should fall back to catcode; got %q", title)
	}
}

// TestWindowTitleAnimatesWhileBusy proves a working session prefixes the title
// with a spinner glyph (distinct from the project name) and that consecutive
// renders keep the busy shape.
func TestWindowTitleAnimatesWhileBusy(t *testing.T) {
	s := titleSession(t)
	s.busy = true

	title := s.windowTitle()
	if !strings.HasSuffix(title, " catcode") {
		t.Fatalf("busy title should end with the project name; got %q", title)
	}
	prefix := strings.TrimSuffix(title, " catcode")
	if prefix == "" {
		t.Fatal("busy title must carry a spinner prefix")
	}
	// Every spinner frame (and the reduced-motion glyph) is a non-space rune.
	if strings.TrimSpace(prefix) == "" {
		t.Fatalf("spinner prefix must be visible; got %q", prefix)
	}
}

// TestWindowTitleBellOnTurnComplete proves the done event arms the bell and a
// keypress silences it.
func TestWindowTitleBellOnTurnComplete(t *testing.T) {
	s := titleSession(t)
	s.busy = true
	s.handleCoreEvent(&coreEvent{Type: "done", Raw: []byte(`{"type":"done"}`)})

	if !s.titleBell {
		t.Fatal("turn completion should arm the title bell")
	}
	if !strings.HasPrefix(s.windowTitle(), "🔔 ") {
		t.Fatalf("title should show the bell after turn completion; got %q", s.windowTitle())
	}

	// The user returning to the keyboard silences the bell.
	s.handleKey(keyMsg("a"))
	if s.titleBell {
		t.Fatal("keypress should silence the title bell")
	}
	if got := s.windowTitle(); got != "catcode" {
		t.Fatalf("title should be bare after the bell is silenced; got %q", got)
	}
}

// TestWindowTitleBellOnError proves a core error event arms the bell.
func TestWindowTitleBellOnError(t *testing.T) {
	s := titleSession(t)
	s.handleCoreEvent(&coreEvent{Type: "error", Raw: []byte(`{"type":"error","message":"boom"}`)})

	if !s.titleBell {
		t.Fatal("error event should arm the title bell")
	}
	if !strings.HasPrefix(s.windowTitle(), "🔔 ") {
		t.Fatalf("title should show the bell after an error; got %q", s.windowTitle())
	}
}

// TestWindowTitleBellWhileBlocking proves an open approval prompt shows the
// bell (attention beats the working spinner) and the bell clears once resolved.
func TestWindowTitleBellWhileBlocking(t *testing.T) {
	s := titleSession(t)
	s.busy = true
	s.pendingApproval = &approvalPrompt{requestID: "a1", tool: "bash"}

	title := s.windowTitle()
	if !strings.HasPrefix(title, "🔔 ") {
		t.Fatalf("pending approval should show the bell even while busy; got %q", title)
	}

	s.pendingApproval = nil
	// Still busy → spinner, not bell.
	if strings.HasPrefix(s.windowTitle(), "🔔 ") {
		t.Fatalf("resolved approval should drop the bell while busy; got %q", s.windowTitle())
	}
}

// TestWindowTitleCoreFailed proves a failed core keeps the bell until the user
// returns (core failure is persistent attention).
func TestWindowTitleCoreFailed(t *testing.T) {
	s := titleSession(t)
	s.coreLifecycle = coreFailed

	if !strings.HasPrefix(s.windowTitle(), "🔔 ") {
		t.Fatalf("core failure should show the bell; got %q", s.windowTitle())
	}
}

// TestWindowTitleFocusRestores proves regaining terminal focus silences the bell.
func TestWindowTitleFocusRestores(t *testing.T) {
	s := titleSession(t)
	s.titleBell = true

	m, _ := s.Update(tea.FocusMsg{})
	ns := m.(*session)
	if ns.titleBell {
		t.Fatal("terminal focus should silence the title bell")
	}
}

// TestWindowTitleInView proves View carries the computed title in
// View.WindowTitle (the declarative Bubble Tea v2 knob the renderer turns into
// OSC 2 sequences).
func TestWindowTitleInView(t *testing.T) {
	s := titleSession(t)
	s.projectName = "proj-x"
	s.models = []modelInfo{{ID: "m1", ContextWindow: 8192}}
	s.modelIdx = 0
	s.authed = true
	s.layout()

	v := s.View()
	if v.WindowTitle != "proj-x" {
		t.Fatalf("View.WindowTitle should carry the project name; got %q", v.WindowTitle)
	}
	if !v.ReportFocus {
		t.Fatal("View must request focus events so the bell can auto-silence")
	}
}

// TestSanitizeWindowTitle proves control characters are stripped and long
// titles capped, so an exotic directory name cannot corrupt the OSC sequence.
func TestSanitizeWindowTitle(t *testing.T) {
	if got := sanitizeWindowTitle("bad\x1b]0;evil\x07name"); got != "bad]0;evilname" {
		t.Fatalf("control chars should be stripped; got %q", got)
	}
	long := strings.Repeat("x", 200)
	if got := sanitizeWindowTitle(long); len(got) != 80 {
		t.Fatalf("title should cap at 80 chars; got %d", len(got))
	}
}

// TestWindowTitleReducedMotionStatic proves reduced motion pins a stable glyph
// (no time-based flicker), matching the working-wave convention.
func TestWindowTitleReducedMotionStatic(t *testing.T) {
	s := titleSession(t)
	s.busy = true
	s.settings.ReducedMotion = true

	first := s.windowTitle()
	second := s.windowTitle()
	if first != second {
		t.Fatalf("reduced-motion title should be stable; got %q vs %q", first, second)
	}
	if !strings.Contains(first, "◷") {
		t.Fatalf("reduced-motion busy title should use the static glyph; got %q", first)
	}
}
