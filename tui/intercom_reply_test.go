package main

import (
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
)

// TestIntercomEmptyEnterNudges guards against the regression where hitting
// Enter on an empty intercom reply was a SILENT no-op. The banner reads
// "↵ reply", so users pressed Enter expecting it to reply, got nothing, then
// Esc'd out with the "[no reply]" nudge — see the worker run that failed at
// the contact_supervisor timeout. Empty Enter must now pulse a "type a reply"
// hint and must NOT send an intercom_reply (Esc still owns the "[no reply]").
func TestIntercomEmptyEnterNudges(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.authed = true
	s.pendingIntercom = &intercomPrompt{
		requestID: "ask-1",
		from:      "worker",
		reason:    "need_decision",
		message:   "A or B?",
	}
	s.input.SetValue("")
	s.input.Focus()
	s.layout()

	// Empty Enter: must not clear the prompt (no reply sent) and must arm the nudge.
	s.handleKey(tea.KeyMsg{Type: tea.KeyEnter})
	if s.pendingIntercom == nil {
		t.Fatal("empty Enter must NOT send an intercom_reply (pendingIntercom was cleared)")
	}
	if s.intercomNudge.IsZero() {
		t.Fatal("empty Enter should arm the intercomNudge hint")
	}
	// The banner should surface the nudge while it is armed.
	if got := s.renderIntercomBanner(); !strings.Contains(got, "type a reply") {
		t.Fatalf("banner should show the nudge hint while armed; got %q", got)
	}
}

// TestIntercomTypedEnterReplies ensures a non-empty Enter still takes the reply
// path: clears the prompt and the input (sendCore is a no-op without a core).
func TestIntercomTypedEnterReplies(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.authed = true
	s.pendingIntercom = &intercomPrompt{
		requestID: "ask-1",
		from:      "worker",
		reason:    "need_decision",
		message:   "A or B?",
	}
	s.input.SetValue("A")
	s.input.Focus()
	s.layout()

	s.handleKey(tea.KeyMsg{Type: tea.KeyEnter})
	if s.pendingIntercom != nil {
		t.Fatal("typed Enter should send the reply and clear pendingIntercom")
	}
	if s.input.Value() != "" {
		t.Fatalf("input should be cleared after sending the reply; got %q", s.input.Value())
	}
}
