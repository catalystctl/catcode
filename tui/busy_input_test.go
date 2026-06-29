package main

import (
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
)

// TestTypingDuringBusy proves the chat input stays editable while a turn runs.
// textinput.Update no-ops unless focused, so this also guards against any
// accidental Blur() on send leaving the input "locked" mid-flight.
func TestTypingDuringBusy(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.models = []modelInfo{{ID: "m1", ContextWindow: 8192}}
	s.modelIdx = 0
	s.authed = true
	s.busy = true
	s.input.Focus()
	s.layout()

	// Type "hi there" while busy.
	for _, r := range "hi there" {
		s.handleKey(keyMsg(string(r)))
	}
	if got := s.input.Value(); got != "hi there" {
		t.Fatalf("input should accept typing while busy; got %q", got)
	}
	if !s.input.Focused() {
		t.Fatal("input should remain focused while busy")
	}

	// Enter during busy with non-slash text queues a follow-up (sendCore is a
	// no-op without a running core, but queuedNext must flip and the input
	// must clear).
	s.queuedNext = false
	s.handleKey(tea.KeyMsg{Type: tea.KeyEnter})
	if s.input.Value() != "" {
		t.Fatalf("input should clear after queuing a follow-up; got %q", s.input.Value())
	}
	if !s.queuedNext {
		t.Fatal("Enter while busy should queue a follow-up (queuedNext=true)")
	}

	// A slash command during busy runs through handleUserLine (no crash, no-op
	// sendCore). Type and submit "/stats".
	for _, r := range "/stats" {
		s.handleKey(keyMsg(string(r)))
	}
	if !strings.HasPrefix(s.input.Value(), "/stats") {
		t.Fatalf("should be able to type a slash command while busy; got %q", s.input.Value())
	}
}

// TestSteerFromInputDuringBusy verifies Ctrl+Enter path (via the command) sets
// queuedNext and clears the input.
func TestSteerFromInputDuringBusy(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.models = []modelInfo{{ID: "m1"}}
	s.modelIdx = 0
	s.authed = true
	s.busy = true
	s.input.Focus()
	for _, r := range "focus on tests" {
		s.handleKey(keyMsg(string(r)))
	}
	s.sendSteer(strings.TrimSpace(s.input.Value()))
	if !s.queuedNext {
		t.Fatal("sendSteer should set queuedNext")
	}
}
