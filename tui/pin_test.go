package main

import (
	"encoding/json"
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
)

// nopWriteCloser is a non-nil io.WriteCloser sink so sendCore's coreIn-nil guard
// passes; the actual bytes are intercepted via stdinCh.
type nopWriteCloser struct{}

func (nopWriteCloser) Write(p []byte) (int, error) { return len(p), nil }
func (nopWriteCloser) Close() error                 { return nil }

// wireCoreStub gives the session a capture channel for sendCore commands.
func wireCoreStub(s *session) {
	s.coreIn = nopWriteCloser{}
	s.stdinCh = make(chan []byte, 16)
}

// sentType reads one command off the stub channel and returns its "type".
func sentType(s *session) string {
	select {
	case b := <-s.stdinCh:
		var m map[string]any
		if json.Unmarshal(b, &m) == nil {
			if t, ok := m["type"].(string); ok {
				return t
			}
		}
	default:
	}
	return ""
}

func newBusySession() *session {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.models = []modelInfo{{ID: "m1", ContextWindow: 8192}}
	s.modelIdx = 0
	s.authed = true
	s.busy = true
	s.input.Focus()
	wireCoreStub(s)
	s.layout()
	return s
}

// TestEscDropsQueuedFollowUp: with a follow-up queued, Esc must dequeue it
// (clear_queue) and leave the in-flight turn running — NOT abort it.
func TestEscDropsQueuedFollowUp(t *testing.T) {
	s := newBusySession()
	s.queued = &queuedMsg{kind: "follow-up", text: "then do X"}
	s.queuedNext = true

	s.handleKey(tea.KeyMsg{Type: tea.KeyEsc})

	if s.queued != nil {
		t.Fatalf("Esc should clear the queued message; still set: %+v", s.queued)
	}
	if s.queuedNext {
		t.Fatal("Esc should clear queuedNext when dequeueing")
	}
	if !s.busy {
		t.Fatal("Esc must NOT abort the in-flight turn when a follow-up is queued")
	}
	if got := sentType(s); got != "clear_queue" {
		t.Fatalf("Esc with a queued follow-up should send clear_queue, got %q", got)
	}
}

// TestEscAbortsWhenNothingQueued: with an empty queue, Esc aborts as before.
func TestEscAbortsWhenNothingQueued(t *testing.T) {
	s := newBusySession()
	s.queued = nil
	s.queuedNext = false

	s.handleKey(tea.KeyMsg{Type: tea.KeyEsc})

	if got := sentType(s); got != "abort" {
		t.Fatalf("Esc with nothing queued should send abort, got %q", got)
	}
}

// TestEscDropsQueuedSteer: a queued steer also dequeues on Esc (the in-flight
// turn was already interrupted by the steer itself).
func TestEscDropsQueuedSteer(t *testing.T) {
	s := newBusySession()
	s.queued = &queuedMsg{kind: "steer", text: "refocus"}
	s.queuedNext = true

	s.handleKey(tea.KeyMsg{Type: tea.KeyEsc})

	if s.queued != nil {
		t.Fatalf("Esc should clear the queued steer; still set: %+v", s.queued)
	}
	if got := sentType(s); got != "clear_queue" {
		t.Fatalf("Esc with a queued steer should send clear_queue, got %q", got)
	}
}

// TestCaptureTodosFromToolCall: a todo_write tool_call pins the latest list so
// the always-visible panel reflects current state.
func TestCaptureTodosFromToolCall(t *testing.T) {
	s := newBusySession()
	args := `{"todos":[{"subject":"A","status":"completed"},{"subject":"B","status":"in_progress"},{"subject":"C","status":"pending"}]}`
	raw, _ := json.Marshal(map[string]any{
		"type": "tool_call",
		"id":   "c1",
		"name": "todo_write",
		"args": args, // marshalled as an escaped JSON string, mirroring the core
	})
	ev := &coreEvent{Type: "tool_call", Raw: raw}
	s.handleCoreEvent(ev)

	if len(s.todos) != 3 {
		t.Fatalf("expected 3 captured todos, got %d", len(s.todos))
	}
	if get(s.todos[1], "subject") != "B" || get(s.todos[1], "status") != "in_progress" {
		t.Fatalf("second todo mismatch: %+v", s.todos[1])
	}
	if h := s.todoPanelHeight(); h == 0 {
		t.Fatal("todo panel should be visible after capture")
	}
	panel := s.renderTodoPanel()
	if !strings.Contains(panel, "tasks") || !strings.Contains(panel, "B") {
		t.Fatalf("todo panel should render header + rows; got:\n%s", panel)
	}
}

// TestCaptureTodosLatestWins: a second todo_write replaces the pinned list.
func TestCaptureTodosLatestWins(t *testing.T) {
	s := newBusySession()
	first := `{"todos":[{"subject":"old","status":"pending"}]}`
	raw, _ := json.Marshal(map[string]any{"type": "tool_call", "id": "c1", "name": "todo_write", "args": first})
	s.handleCoreEvent(&coreEvent{Type: "tool_call", Raw: raw})

	second := `{"todos":[{"subject":"new","status":"completed"}]}`
	raw2, _ := json.Marshal(map[string]any{"type": "tool_call", "id": "c2", "name": "todo_write", "args": second})
	s.handleCoreEvent(&coreEvent{Type: "tool_call", Raw: raw2})

	if len(s.todos) != 1 || get(s.todos[0], "subject") != "new" {
		t.Fatalf("latest todo_write should win; got %+v", s.todos)
	}
}

// TestQueueBannerLabelsKind: the pinned queue banner names follow-up vs steer.
func TestQueueBannerLabelsKind(t *testing.T) {
	s := newBusySession()

	s.queued = &queuedMsg{kind: "follow-up", text: "hello"}
	if b := s.renderQueueBanner(); !strings.Contains(b, "follow-up") {
		t.Fatalf("follow-up banner should label kind; got:\n%s", b)
	}

	s.queued = &queuedMsg{kind: "steer", text: "hello"}
	if b := s.renderQueueBanner(); !strings.Contains(b, "steer") {
		t.Fatalf("steer banner should label kind; got:\n%s", b)
	}

	s.queued = nil
	if b := s.renderQueueBanner(); b != "" {
		t.Fatalf("no banner when nothing queued; got:\n%s", b)
	}
}
