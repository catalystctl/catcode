package main

import (
	"testing"
)

// TestBlockingInputOpenCoversModals locks in the fix for the modal typing-lag
// bug: a keyboard-intercepting overlay isn't only the ask/sudo/approval
// flyout — ANY modal (command palette, settings edit, OAuth code box, …)
// owns the keyboard too. blockingInputOpen is the single gate the
// busy-frame (10×/s View) and tickMsg/streamRefreshMsg renderBlocks churn
// pause behind; if it forgets modals, typing in a modal while the agent is
// busy streams lags by seconds per keystroke (the same root cause as the
// ask-flyout lag, on the modal path).
func TestBlockingInputOpenCoversModals(t *testing.T) {
	s := initialSession()

	// No overlay → not blocking.
	if s.blockingInputOpen() {
		t.Fatal("blockingInputOpen should be false with no flyout or modal")
	}

	// Each modal kind counts as blocking input.
	for _, k := range []modalKind{
		modalCommand, modalSettings, modalOauthCode, modalProviders, modalSearchKey,
	} {
		s.modal.kind = k
		if !s.blockingInputOpen() {
			t.Fatalf("modal %v open should count as blocking input", k)
		}
	}

	// A flyout alone (no modal) still counts, and composes with a modal.
	s.modal.kind = modalNone
	s.pendingAsk = &askPrompt{}
	if !s.blockingInputOpen() {
		t.Fatal("pendingAsk should count as blocking input")
	}
}

// TestBusyFrameStopsWhileModalOpen verifies the ~10×/s re-render storm is
// paused while a modal owns the keyboard, and re-arms within the normal
// busy cycle once the modal closes. Without this, every busyFrameMsg drives
// a full View() (transcript + overlay) on bubbletea's single thread and
// keystrokes into the modal queue behind the flood.
func TestBusyFrameStopsWhileModalOpen(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.busy = true
	s.busyFrameActive = true // simulate a turn already in flight

	// Modal open → no re-arm; the storm drains.
	s.modal.kind = modalCommand
	_, cmd := s.Update(busyFrameMsg{})
	if cmd != nil {
		t.Fatalf("busyFrameMsg should not re-arm while a modal is open; got cmd %T", cmd)
	}
	if s.busyFrameActive {
		t.Fatal("busyFrameActive must be false while a modal is open")
	}

	// Modal closed + still busy → storm re-arms (the verified resume path).
	s.modal.kind = modalNone
	_, cmd = s.Update(busyFrameMsg{})
	if cmd == nil {
		t.Fatal("busyFrameMsg should re-arm the busy-frame after the modal closes")
	}
	if !s.busyFrameActive {
		t.Fatal("busyFrameActive should be true once busy + no modal")
	}
}

// TestStreamRefreshSkippedWhileModalOpen covers the streaming-while-modal
// case the ask-flyout fix left as a caveat: a modal does NOT pause the agent
// (unlike ask/sudo/approval), so the agent can keep streaming while the user
// types. Each streamRefreshMsg would rebuild the whole transcript behind the
// modal — pure waste that starves typing. It must skip refresh() while a modal
// is open and resume (rebuild) once the modal closes.
func TestStreamRefreshSkippedWhileModalOpen(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.layout()

	// Seed a transcript and capture its rendered base.
	s.push(blkUser).appendText("first message")
	s.refresh()
	base := s.transcriptBase
	if base == "" {
		t.Fatal("expected a non-empty transcript base after refresh")
	}

	// Add content WITHOUT refreshing, so a later refresh() is observable.
	s.push(blkUser).appendText("second message")
	if got := s.transcriptBase; got != base {
		t.Fatalf("transcriptBase must not change from a bare push; got %q want %q", got, base)
	}

	// Modal open → streamRefreshMsg must NOT rebuild (skip refresh).
	s.modal.kind = modalCommand
	s.Update(streamRefreshMsg{})
	if s.transcriptBase != base {
		t.Fatalf("streamRefreshMsg should skip refresh while a modal is open; base changed to %q (want %q)", s.transcriptBase, base)
	}

	// Modal closed → streamRefreshMsg rebuilds, picking up the second block.
	s.modal.kind = modalNone
	s.Update(streamRefreshMsg{})
	if s.transcriptBase == base {
		t.Fatal("streamRefreshMsg should refresh after the modal closes; transcriptBase unchanged")
	}
}
