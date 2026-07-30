package main

import (
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"
)

// Regression test for the "ask flyout text input is catastrophically laggy"
// report (one letter every ~30s, Ctrl+C unusable).
//
// Root cause: bubbles v2 textinput.Update returns a cursor-blink cmd on every
// cursor-moving keystroke. In bubbles v2 that blink cmd BLOCKS when invoked —
// it waits on a context deadline (BlinkSpeed = 530ms). pumpHuhAsk used to
// drain the form's cmds SYNCHRONOUSLY on the UI goroutine
// (for i := 0; i < 8 && cmd != nil; i++ { next := cmd(); ... }), and feeding
// each resulting BlinkMsg back into the form re-arms the blink, so every
// keystroke blocked the single-threaded loop for up to ~8×530ms ≈ 4.2s.
// Queued keystrokes compounded (a burst of letters = tens of seconds of
// blocked loop) and even Ctrl+C queued behind the flood — the reported
// lockup. Select pickers were unaffected because they never return blink
// cmds, which is why the lag appeared ONLY on text inputs ("other"/custom
// reply boxes).
//
// The fix: pumpHuhAsk must NOT execute cmds inline — it returns them for the
// bubbletea runtime to execute asynchronously (the same pattern as
// handleSudoKey / the main chat input), and unhandled msgs (BlinkMsg etc.)
// are forwarded back to the form via the main Update's default case.
func TestAskTextInputKeystrokeDoesNotBlock(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.layout()

	s.handleCoreEvent(askRequestEvent(t, "ask-lag",
		`[{"id":"name","prompt":"Your name?","type":"text","required":false}]`))
	if s.pendingAsk == nil {
		t.Fatal("ask_request must open the flyout")
	}

	key := tea.KeyPressMsg{Code: 'x', Text: "x"}
	start := time.Now()
	_, cmd := s.handleAskKey(key)
	elapsed := time.Since(start)

	// The blink cmd MUST be returned, not run: executing it blocks 530ms by
	// design, so the runtime runs it on its own goroutine. The old code ran
	// it inline up to 8× ≈ 4.2s per keystroke; 1s discriminates with margin
	// while the healthy path is microseconds.
	if elapsed > time.Second {
		t.Fatalf("keystroke blocked the UI goroutine for %v — synchronous cmd "+
			"drain regression (the textinput cursor-blink cmd blocks 530ms when "+
			"invoked; it must be returned for async runtime execution, not run inline)", elapsed)
	}
	if cmd == nil {
		t.Fatal("expected the form's cmd (cursor blink) to be returned for async execution")
	}
	if got := s.pendingAsk.fieldValues[0]; got != "x" {
		t.Fatalf("typed char not applied: fieldValues[0] = %q, want %q", got, "x")
	}
}

// The returned blink cmd resolves (async, ~530ms) into a msg the form needs
// back (cursor.BlinkMsg) so the cursor keeps blinking. Verify the full async
// round-trip: keystroke → cmd → msg → forward to form → non-nil re-arm cmd,
// and that the forward path itself never blocks.
func TestAskBlinkMsgRoundTripStaysAsync(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.layout()

	s.handleCoreEvent(askRequestEvent(t, "ask-lag-2",
		`[{"id":"name","prompt":"Your name?","type":"text","required":false}]`))
	if s.pendingAsk == nil {
		t.Fatal("ask_request must open the flyout")
	}

	_, cmd := s.handleAskKey(tea.KeyPressMsg{Code: 'x', Text: "x"})
	if cmd == nil {
		t.Fatal("keystroke should return the blink cmd")
	}

	// Execute the cmd the way the bubbletea runtime would — on another
	// goroutine, since it blocks until the blink deadline by design.
	msgCh := make(chan tea.Msg, 1)
	go func() { msgCh <- cmd() }()
	var msg tea.Msg
	select {
	case msg = <-msgCh:
	case <-time.After(2 * time.Second):
		t.Fatal("blink cmd never resolved — the cursor would freeze")
	}
	if msg == nil {
		t.Fatal("blink cmd resolved to nil — cursor blink chain broken")
	}

	// Forwarding the resolved msg to the form (what the main Update's default
	// case does while the flyout is open) must be fast and must re-arm.
	start := time.Now()
	rearm := s.pumpHuhAsk(msg)
	if elapsed := time.Since(start); elapsed > time.Second {
		t.Fatalf("forwarding blink msg blocked for %v — must stay async", elapsed)
	}
	if rearm == nil {
		t.Fatal("blink msg should re-arm a follow-up cmd (cursor keeps blinking)")
	}
}
