package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

func TestSessionClaimPreventsConcurrentReuse(t *testing.T) {
	releaseSessionClaim()
	t.Cleanup(releaseSessionClaim)
	path := filepath.Join(t.TempDir(), "session.jsonl")
	if !claimSession(path) {
		t.Fatal("first claim failed")
	}
	// Simulate another process by forgetting our in-memory owner without
	// removing its on-disk lease.
	claimedSessionLock, claimedSessionPath = "", ""
	if claimSession(path) {
		t.Fatal("second claim unexpectedly reused an active session")
	}
}

func TestSessionReservationKeepsOldClaimUntilAcknowledged(t *testing.T) {
	releaseSessionClaim()
	t.Cleanup(releaseSessionClaim)
	dir := t.TempDir()
	oldPath := filepath.Join(dir, "old.jsonl")
	newPath := filepath.Join(dir, "new.jsonl")
	if !claimSession(oldPath) || !reserveSession(newPath) {
		t.Fatal("could not establish old and reserved claims")
	}
	if _, err := os.Stat(oldPath + ".lock"); err != nil {
		t.Fatalf("old claim released before acknowledgement: %v", err)
	}
	if !commitSessionClaim(newPath) {
		t.Fatal("could not commit reserved claim")
	}
	if _, err := os.Stat(oldPath + ".lock"); !os.IsNotExist(err) {
		t.Fatalf("old claim remains after acknowledgement: %v", err)
	}
	if _, err := os.Stat(newPath + ".lock"); err != nil {
		t.Fatalf("new claim missing after acknowledgement: %v", err)
	}
}

func TestDeadProcessSessionLockIsRecovered(t *testing.T) {
	releaseSessionClaim()
	t.Cleanup(releaseSessionClaim)
	path := filepath.Join(t.TempDir(), "crashed.jsonl")
	if err := os.WriteFile(path+".lock", []byte("pid=99999999\n"), 0600); err != nil {
		t.Fatal(err)
	}
	if !claimSession(path) {
		t.Fatal("dead process lock was not recovered")
	}
}

func TestPasteRoutesToSudoWithoutLeakingToComposer(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.input.SetValue("existing draft")
	s.pendingSudo = newSudoPrompt("sudo-1", "sudo true")
	_, _ = s.Update(tea.PasteMsg{Content: "super-secret"})
	if got := s.pendingSudo.input.Value(); got != "super-secret" {
		t.Fatalf("sudo paste = %q", got)
	}
	if got := s.input.Value(); got != "existing draft" {
		t.Fatalf("password leaked into composer/draft changed: %q", got)
	}
}

func TestModalPasteNeverLeaksBehindDialog(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.input.SetValue("draft")
	s.openCommandPalette()
	_, _ = s.Update(tea.PasteMsg{Content: "model"})
	if s.modal.filter != "model" || s.modal.pickerList.FilterValue() != "model" {
		t.Fatalf("modal paste filter: mirror=%q picker=%q", s.modal.filter, s.modal.pickerList.FilterValue())
	}
	if s.input.Value() != "draft" {
		t.Fatalf("hidden composer changed: %q", s.input.Value())
	}
}

func TestApprovalSuspendsAndRestoresDraft(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.input.SetValue("careful follow-up")
	s.handleCoreEvent(&coreEvent{Type: "approval_request", Raw: json.RawMessage(`{"type":"approval_request","request_id":"a1","tool":"bash","args":"{}"}`)})
	if s.input.Value() != "" {
		t.Fatalf("approval should suspend draft, got %q", s.input.Value())
	}
	_, _ = s.handleKey(keyMsg("y"))
	if got := s.input.Value(); got != "careful follow-up" {
		t.Fatalf("draft was not restored: %q", got)
	}
}

func TestIntercomQuestionsQueueAndRestoreDraft(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.input.SetValue("original")
	for _, id := range []string{"q1", "q2"} {
		raw, _ := json.Marshal(map[string]any{"type": "intercom_message", "id": id, "from": id, "reason": "need_decision", "message": "answer"})
		s.handleCoreEvent(&coreEvent{Type: "intercom_message", Raw: raw})
	}
	if s.pendingIntercom == nil || s.pendingIntercom.requestID != "q1" || len(s.intercomQueue) != 1 {
		t.Fatalf("intercom queue state: active=%+v queued=%d", s.pendingIntercom, len(s.intercomQueue))
	}
	s.input.SetValue("first")
	_, _ = s.handleKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.pendingIntercom == nil || s.pendingIntercom.requestID != "q2" {
		t.Fatalf("second question not advanced: %+v", s.pendingIntercom)
	}
	s.input.SetValue("second")
	_, _ = s.handleKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.pendingIntercom != nil || len(s.intercomQueue) != 0 || s.input.Value() != "original" {
		t.Fatalf("queue did not drain/restore: active=%+v queued=%d draft=%q", s.pendingIntercom, len(s.intercomQueue), s.input.Value())
	}
}

func TestBackpressuredSendPreservesDraftAndIdleState(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.authed = true
	s.models = []modelInfo{{ID: "m"}}
	s.coreIn = nopWriteCloser{}
	s.stdinCh = make(chan []byte, 1)
	s.stdinCh <- []byte("occupied")
	s.input.SetValue("do not lose me")
	_, _ = s.handleKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if !strings.Contains(s.input.Value(), "do not lose me") {
		t.Fatalf("draft lost under backpressure: %q", s.input.Value())
	}
	if s.busy {
		t.Fatal("rejected dispatch must not enter busy state")
	}
}

func TestStaleCoreGenerationEventIsIgnored(t *testing.T) {
	s := initialSession()
	s.coreStartGen = 2
	s.authed = false
	raw := json.RawMessage(`{"type":"authed","ok":true}`)
	_, _ = s.Update(coreEventMsg{event: &coreEvent{Type: "authed", Raw: raw}, gen: 1})
	if s.authed {
		t.Fatal("stale generation mutated current session")
	}
}

func TestSessionActionsDoNotHijackFilterTyping(t *testing.T) {
	s := initialSession()
	s.sessionList = []sessionEntry{{Path: "/tmp/one.jsonl", Title: "one"}}
	s.openSessionsPicker()
	_, _ = s.handleListKey(keyMsg("p"))
	if s.modal.filter != "p" || s.sessionList[0].Pinned {
		t.Fatalf("typing p should filter, not pin: filter=%q pinned=%v", s.modal.filter, s.sessionList[0].Pinned)
	}
}

func TestSelectingCurrentSessionIsNoOp(t *testing.T) {
	s := initialSession()
	s.coreIn = nopWriteCloser{}
	s.stdinCh = make(chan []byte, 1)
	s.sessionList = []sessionEntry{{Path: "/tmp/current.jsonl", Title: "current", Current: true}}
	s.openSessionsPicker()
	_, _ = s.executeListSelect(0)
	if len(s.stdinCh) != 0 {
		t.Fatal("selecting the current session dispatched a destructive reload")
	}
}
