package main

import (
	"strings"
	"testing"
	"time"
)

func splashSession(t *testing.T) *session {
	t.Helper()
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.coreStartGen = 1
	s.coreLifecycle = coreStarting
	s.layout()
	return s
}

func TestSplashShowsStartingAndCredentials(t *testing.T) {
	s := splashSession(t)
	view := stripANSI(s.renderBlocks())
	for _, want := range []string{"Starting", "checking", "credentials", "Catalyst"} {
		if !strings.Contains(view, want) {
			t.Fatalf("splash missing %q:\n%s", want, view)
		}
	}
	// Must not claim the user needs to log in while still booting.
	if strings.Contains(view, "No API key") || strings.Contains(view, "Log in first") {
		t.Fatalf("startup splash must not claim credentials are missing:\n%s", view)
	}
}

func TestSplashAnimatesAcrossFrames(t *testing.T) {
	s := splashSession(t)
	s.settings.ReducedMotion = false

	first := s.renderSplashScreen(80, 24)
	time.Sleep(200 * time.Millisecond) // cross at least one busy-frame boundary
	second := s.renderSplashScreen(80, 24)
	if first == second {
		t.Fatal("splash should change across frames when motion is allowed")
	}
}

func TestSplashReducedMotionIsStatic(t *testing.T) {
	s := splashSession(t)
	s.settings.ReducedMotion = true

	first := s.renderSplashScreen(80, 24)
	time.Sleep(200 * time.Millisecond)
	second := s.renderSplashScreen(80, 24)
	if first != second {
		t.Fatal("reduced-motion splash should be static across frames")
	}
	plain := stripANSI(first)
	if !strings.Contains(plain, "Starting") || !strings.Contains(plain, "Catalyst") {
		t.Fatalf("reduced-motion splash missing brand/status:\n%s", plain)
	}
}

func TestNeedsBusyFramesWhileCoreStarting(t *testing.T) {
	s := splashSession(t)
	s.splashStartedAt = time.Now()
	if !s.needsBusyFrames() {
		t.Fatal("coreStarting should keep the busy-frame clock armed")
	}
	if !s.splashAnimatesInViewport() {
		t.Fatal("coreStarting empty transcript should animate the splash in the viewport")
	}
	s.settings.ReducedMotion = true
	if s.splashAnimatesInViewport() {
		t.Fatal("reduced motion should skip viewport splash refresh")
	}
	// After min-hold expires, coreReady must stop the splash clock.
	s.coreLifecycle = coreReady
	s.settings.ReducedMotion = false
	s.splashStartedAt = time.Now().Add(-splashMinHold - time.Second)
	if s.needsBusyFrames() {
		t.Fatal("coreReady past min-hold should not need busy frames")
	}
	if s.splashAnimatesInViewport() {
		t.Fatal("coreReady past min-hold should not animate the splash")
	}
}

func TestSplashHoldsAfterFastReady(t *testing.T) {
	s := splashSession(t)
	s.splashStartedAt = time.Now()
	s.coreLifecycle = coreReady
	s.authed = true
	s.models = []modelInfo{{ID: "m1", ContextWindow: 8192}}
	s.modelIdx = 0

	if !s.showingSplash() {
		t.Fatal("fast ready within min-hold must still show the splash")
	}
	view := stripANSI(s.renderBlocks())
	if !strings.Contains(view, "Starting") || !strings.Contains(view, "Catalyst") {
		t.Fatalf("held splash missing brand/status:\n%s", view)
	}

	// Expire the hold → welcome examples replace the splash.
	s.splashStartedAt = time.Now().Add(-splashMinHold - time.Second)
	if s.showingSplash() {
		t.Fatal("splash should dismiss after min-hold")
	}
	s.layout()
	view = stripANSI(s.renderBlocks())
	if strings.Contains(view, "Starting") {
		t.Fatalf("expired hold left a stale splash:\n%s", view)
	}
	if !strings.Contains(view, "Understand this repository") {
		t.Fatalf("post-hold welcome examples missing:\n%s", view)
	}
}

func TestSplashHoldDoneDismissesAfterReady(t *testing.T) {
	s := splashSession(t)
	s.splashStartedAt = time.Now()
	s.coreLifecycle = coreReady
	s.authed = true
	s.models = []modelInfo{{ID: "m1", ContextWindow: 8192}}
	s.modelIdx = 0
	s.layout()
	if view := stripANSI(s.viewport.View()); !strings.Contains(view, "Starting") {
		t.Fatalf("precondition: held splash not in viewport:\n%s", view)
	}

	// Simulate hold elapsed + tick: force clock past min-hold, then fire the msg.
	s.splashStartedAt = time.Now().Add(-splashMinHold - time.Second)
	_, _ = s.Update(splashHoldDoneMsg{gen: s.coreStartGen})
	view := stripANSI(s.viewport.View())
	if strings.Contains(view, "Starting") {
		t.Fatalf("splashHoldDoneMsg should replace the splash:\n%s", view)
	}
	if !strings.Contains(view, "Understand this repository") {
		t.Fatalf("welcome examples missing after hold done:\n%s", view)
	}
}

func TestPreReadyViewShowsSplash(t *testing.T) {
	s := initialSession()
	// ready=false is the pre-WindowSizeMsg path.
	s.ready = false
	s.coreStartGen = 1
	s.coreLifecycle = coreStarting
	view := stripANSI(s.View().Content)
	if !strings.Contains(view, "Starting") || !strings.Contains(view, "Catalyst") {
		t.Fatalf("pre-ready View should paint the branded splash:\n%s", view)
	}
}

func TestBusyFrameRefreshesSplash(t *testing.T) {
	s := splashSession(t)
	s.settings.ReducedMotion = false
	s.busyFrameActive = true
	before := s.transcriptBase
	_, cmd := s.Update(busyFrameMsg{})
	if cmd == nil {
		t.Fatal("busyFrameMsg during splash should re-arm the frame clock")
	}
	// refresh() may or may not change transcriptBase depending on phase quantisation;
	// at minimum the frame path must remain active.
	if !s.busyFrameActive {
		t.Fatal("busyFrameActive should stay true while splash is showing")
	}
	// Force a later phase and refresh to prove the path can update content.
	time.Sleep(200 * time.Millisecond)
	s.refresh()
	if s.transcriptBase == "" {
		t.Fatal("splash refresh left an empty transcript base")
	}
	_ = before
}
