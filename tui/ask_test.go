package main

import (
	"encoding/json"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

// askRequestEvent builds a *coreEvent simulating the core's `ask_request`
// wire event (type + raw JSON carrying request_id + the questions array).
func askRequestEvent(t *testing.T, requestID, questions string) *coreEvent {
	t.Helper()
	raw, err := json.Marshal(map[string]any{
		"request_id": requestID,
		"questions":  json.RawMessage(questions),
	})
	if err != nil {
		t.Fatalf("marshal ask_request: %v", err)
	}
	return &coreEvent{Type: "ask_request", Raw: raw}
}

const twoQs = `[
	{"id":"isolation","prompt":"Which isolation?","type":"select","options":["Ephemeral","Persistent"],"required":true},
	{"id":"note","prompt":"Any notes?","type":"text","required":false,"placeholder":"optional"}
]`

// TestAskRequestSetsFlyout is the core regression: the TUI defined
// parseAskRequest / handleAskKey / renderAskOverlay in ask.go but NEVER wired
// the `ask_request` event into the dispatch switch — so the model's `ask` call
// appeared as a plain tool block ("▸ ask (...)") with NO flyout and the core
// blocked forever on an answer that never came. Now ask_request must populate
// s.pendingAsk with the parsed questions.
func TestAskRequestSetsFlyout(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.layout()

	s.handleCoreEvent(askRequestEvent(t, "ask-1", twoQs))

	if s.pendingAsk == nil {
		t.Fatal("ask_request event must set s.pendingAsk (the flyout was never opened — the original bug)")
	}
	if s.pendingAsk.requestID != "ask-1" {
		t.Fatalf("requestID = %q, want ask-1", s.pendingAsk.requestID)
	}
	if len(s.pendingAsk.questions) != 2 {
		t.Fatalf("expected 2 questions parsed, got %d", len(s.pendingAsk.questions))
	}
	q0 := s.pendingAsk.questions[0]
	if q0.id != "isolation" || q0.qtype != "select" || len(q0.options) != 2 || !q0.required {
		t.Fatalf("first question mis-parsed: %+v", q0)
	}
	q1 := s.pendingAsk.questions[1]
	if q1.id != "note" || q1.qtype != "text" || q1.required {
		t.Fatalf("second question mis-parsed: %+v", q1)
	}
}

// TestAskRequestRendersFlyout guards the render wiring (the third missing
// piece): with a pending ask the centered overlay must surface the question
// prompt and select options. Without the renderAskOverlay call in the view
// assembly the flyout state was set but invisible.
func TestAskRequestRendersFlyout(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.layout()

	base := "BASE\nVIEW"
	// No pending ask → overlay is a passthrough.
	if got := s.renderAskOverlay(base); got != base {
		t.Fatalf("renderAskOverlay should be a no-op when nothing is pending; got %q", got)
	}

	s.handleCoreEvent(askRequestEvent(t, "ask-1", twoQs))
	if s.pendingAsk == nil {
		t.Fatal("setup: ask_request did not set pendingAsk")
	}
	got := stripANSI(s.renderAskOverlay(base))
	if !strings.Contains(got, "Which isolation?") {
		t.Fatalf("ask flyout should render the question prompt; got:\n%s", got)
	}
	if !strings.Contains(got, "Ephemeral") {
		t.Fatalf("ask flyout should render the select options; got:\n%s", got)
	}
}

// TestAskSubmitSendsReplyAndClears guards the key-dispatch wiring: with the
// flyout open, Enter must submit the default selection (first option), dispatch
// ask_reply, and clear the prompt. sendCore is a no-op without a real core.
func TestAskSubmitSendsReplyAndClears(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.layout()

	s.handleCoreEvent(askRequestEvent(t, "ask-1",
		`[{"id":"isolation","prompt":"Which?","type":"select","options":["Ephemeral","Persistent"],"required":true}]`))
	if s.pendingAsk == nil {
		t.Fatal("setup: ask_request did not set pendingAsk")
	}

	// Enter on a select defaults to the first option ("Ephemeral").
	s.handleKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.pendingAsk != nil {
		t.Fatal("Enter should submit the answer and clear s.pendingAsk")
	}
}

// TestAskSkipClears verifies Esc skips the prompt (sends null answers) and
// clears the flyout so the model isn't wedged waiting on ask_reply.
func TestAskSkipClears(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.layout()

	s.handleCoreEvent(askRequestEvent(t, "ask-1",
		`[{"id":"isolation","prompt":"Which?","type":"select","options":["A","B"],"required":true}]`))
	if s.pendingAsk == nil {
		t.Fatal("setup: ask_request did not set pendingAsk")
	}

	s.handleKey(tea.KeyPressMsg{Code: tea.KeyEsc})
	if s.pendingAsk != nil {
		t.Fatal("Esc should skip the ask prompt and clear s.pendingAsk")
	}
}

// TestAskNavigationKeys guards the fix for "up/down don't move between
// questions": handleAskKey used unregistered action names ("next_field"/
// "down"/"prev_field"/"up") so s.kb returned false for all of them and
// navigation never fired. Now ↓/↑/Tab/Shift+Tab must move focus between
// questions.
func TestAskNavigationKeys(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	// Use default keybinds so the test is isolated from the user's settings
	// (e.g. a user may have disabled nav_up_alt, which would break the k case).
	s.keybinds = defaultKeybinds()
	s.layout()

	s.handleCoreEvent(askRequestEvent(t, "ask-1", twoQs)) // 2 questions
	if s.pendingAsk == nil {
		t.Fatal("setup: ask_request did not set pendingAsk")
	}
	if s.pendingAsk.focusIdx != 0 {
		t.Fatalf("focus should start at 0; got %d", s.pendingAsk.focusIdx)
	}

	// ↓ moves to question 2.
	s.handleKey(tea.KeyPressMsg{Code: tea.KeyDown})
	if s.pendingAsk.focusIdx != 1 {
		t.Fatalf("Down should move focus 0→1; got %d", s.pendingAsk.focusIdx)
	}
	// ↓ at the last question clamps (no wrap).
	s.handleKey(tea.KeyPressMsg{Code: tea.KeyDown})
	if s.pendingAsk.focusIdx != 1 {
		t.Fatalf("Down at last should clamp; got %d", s.pendingAsk.focusIdx)
	}
	// ↑ moves back to question 1.
	s.handleKey(tea.KeyPressMsg{Code: tea.KeyUp})
	if s.pendingAsk.focusIdx != 0 {
		t.Fatalf("Up should move focus 1→0; got %d", s.pendingAsk.focusIdx)
	}
	// ↑ at the first question clamps.
	s.handleKey(tea.KeyPressMsg{Code: tea.KeyUp})
	if s.pendingAsk.focusIdx != 0 {
		t.Fatalf("Up at first should clamp; got %d", s.pendingAsk.focusIdx)
	}
	// Tab also moves forward.
	s.handleKey(tea.KeyPressMsg{Code: tea.KeyTab})
	if s.pendingAsk.focusIdx != 1 {
		t.Fatalf("Tab should move focus 0→1; got %d", s.pendingAsk.focusIdx)
	}
	// j (nav_down_alt) also moves forward from 0.
	s.pendingAsk.focusIdx = 0
	s.handleKey(tea.KeyPressMsg{Code: 'j', Text: "j"})
	if s.pendingAsk.focusIdx != 1 {
		t.Fatalf("j should move focus 0→1; got %d", s.pendingAsk.focusIdx)
	}
	// k (nav_up_alt) moves back.
	s.handleKey(tea.KeyPressMsg{Code: 'k', Text: "k"})
	if s.pendingAsk.focusIdx != 0 {
		t.Fatalf("k should move focus 1→0; got %d", s.pendingAsk.focusIdx)
	}
}

// TestAskRequiredErrorIsInlineNotSpam guards the UX fix: pressing Enter on an
// empty required text field must NOT log a transcript error line each time
// (the old behavior spammed "✗ required" per Enter). Instead the error is shown
// transiently inside the flyout (a.errMsg) and the prompt stays open.
func TestAskRequiredErrorIsInlineNotSpam(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.layout()

	beforeBlocks := len(s.blocks)
	s.handleCoreEvent(askRequestEvent(t, "ask-1",
		`[{"id":"feat","prompt":"Name one feature you want the ask tool to support.","type":"text","required":true}]`))
	if s.pendingAsk == nil {
		t.Fatal("setup: ask_request did not set pendingAsk")
	}

	// Mash Enter 6× on the empty required text field.
	for i := 0; i < 6; i++ {
		s.handleKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	}

	// The flyout must still be open (submit was blocked by the empty required field).
	if s.pendingAsk == nil {
		t.Fatal("Enter on an empty required field must NOT submit / clear the flyout")
	}
	// The inline error must be set and rendered in the flyout.
	if s.pendingAsk.errMsg == "" {
		t.Fatal("empty required submit should set an inline a.errMsg")
	}
	rendered := stripANSI(s.renderAskBox())
	if !strings.Contains(rendered, s.pendingAsk.errMsg) {
		t.Fatalf("flyout should render the inline error %q; got:\n%s", s.pendingAsk.errMsg, rendered)
	}
	// CRITICAL: no transcript error blocks were appended (the old logError spam).
	// The ask_request logInfo adds one info block; nothing else should appear.
	newBlocks := len(s.blocks) - beforeBlocks
	if newBlocks > 1 {
		t.Fatalf("repeated Enter must not spam the transcript: %d new blocks (want <=1)", newBlocks)
	}

	// Typing clears the stale inline error.
	s.handleKey(tea.KeyPressMsg{Code: 'x', Text: "x"})
	if s.pendingAsk.errMsg != "" {
		t.Fatal("typing should clear the stale inline error")
	}
}
