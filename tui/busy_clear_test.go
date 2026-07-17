package main

import (
	"encoding/json"
	"testing"
)

func TestGoalKeepsBusyCEOPhases(t *testing.T) {
	s := initialSession()
	for _, phase := range []string{"planning", "reviewing", "verifying", "replanning", "deploying", "running", "synthesizing"} {
		s.goalState = &goalStateSnap{ID: "g1", Phase: phase}
		if !s.goalKeepsBusy() {
			t.Fatalf("goalKeepsBusy(%s) = false, want true (web parity)", phase)
		}
		if !goalShowsProgressPanel(phase, false) {
			t.Fatalf("goalShowsProgressPanel(%s) = false, want true", phase)
		}
	}
	s.goalState = &goalStateSnap{ID: "g1", Phase: "plan_ready", AutoDeploy: false}
	if s.goalKeepsBusy() {
		t.Fatal("plan_ready without AutoDeploy must not keep busy")
	}
	s.goalState.AutoDeploy = true
	if !s.goalKeepsBusy() {
		t.Fatal("plan_ready+AutoDeploy must keep busy")
	}
}

func TestDoneKeepsBusyWhileVerifying(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.busy = true
	s.goalState = &goalStateSnap{ID: "g1", Phase: "verifying"}
	s.handleCoreEvent(&coreEvent{Type: "done", Raw: []byte(`{"type":"done"}`)})
	if !s.busy {
		t.Fatal("done during verifying must keep busy (CEO parity)")
	}
}

func TestGoalStateIdleClearsBusy(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.busy = true
	s.goalState = &goalStateSnap{ID: "g1", Phase: "running"}
	s.subProgress = []*subProgressEntry{{runID: "r1", agent: "worker"}}
	idle, _ := json.Marshal(map[string]any{
		"type": "goal_state", "id": "", "phase": "idle",
	})
	s.handleCoreEvent(&coreEvent{Type: "goal_state", Raw: idle})
	if s.busy {
		t.Fatal("goal_state idle must clear busy")
	}
	if s.goalState != nil {
		t.Fatal("goal_state idle must nil goalState")
	}
	if s.subProgress != nil {
		t.Fatal("goal_state idle must clear subProgress")
	}
}

func TestGoalPhaseTerminalSyncsPhaseAndClearsBusy(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.busy = true
	s.goalState = &goalStateSnap{ID: "g1", Phase: "synthesizing", Goal: "ship"}
	phase, _ := json.Marshal(map[string]any{
		"type": "goal_phase", "from": "synthesizing", "to": "done",
	})
	s.handleCoreEvent(&coreEvent{Type: "goal_phase", Raw: phase})
	if s.busy {
		t.Fatal("goal_phase done must clear busy")
	}
	if s.goalState == nil || s.goalState.Phase != "done" {
		t.Fatalf("goal_phase done must sync Phase, got %+v", s.goalState)
	}
	// A later normal turn done must not re-stick via stale synthesizing keep-busy.
	s.busy = true
	s.handleCoreEvent(&coreEvent{Type: "done", Raw: []byte(`{"type":"done"}`)})
	if s.busy {
		t.Fatal("done after terminal goal_phase must clear busy")
	}
}

func TestAbortedClearsPendingAsk(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.busy = true
	s.pendingAsk = parseAskRequest("ask-1", json.RawMessage(`[{"id":"q1","prompt":"hi","type":"text"}]`))
	if s.pendingAsk == nil {
		t.Fatal("setup: parseAskRequest returned nil")
	}
	s.handleCoreEvent(&coreEvent{Type: "aborted", Raw: []byte(`{"type":"aborted"}`)})
	if s.busy {
		t.Fatal("aborted must clear busy")
	}
	if s.pendingAsk != nil {
		t.Fatal("aborted must clear orphan pendingAsk")
	}
}

func TestDoneClearsPendingAsk(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.busy = true
	s.pendingAsk = parseAskRequest("ask-2", json.RawMessage(`[{"id":"q1","prompt":"hi","type":"text"}]`))
	s.handleCoreEvent(&coreEvent{Type: "done", Raw: []byte(`{"type":"done"}`)})
	if s.busy {
		t.Fatal("done must clear busy")
	}
	if s.pendingAsk != nil {
		t.Fatal("done must clear pendingAsk")
	}
}

func TestErrorClearsBusyPreTurn(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.busy = true
	// No cur / prompts — optimistic busy after a failed dispatch.
	raw, _ := json.Marshal(map[string]any{
		"type": "error", "message": "unknown skill: nope",
	})
	s.handleCoreEvent(&coreEvent{Type: "error", Raw: raw})
	if s.busy {
		t.Fatal("pre-turn error must clear busy")
	}
}

func TestErrorKeepsBusyMidTurn(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.busy = true
	s.cur = &block{kind: blkAssistant}
	raw, _ := json.Marshal(map[string]any{
		"type": "error", "message": "tool failed: edit",
	})
	s.handleCoreEvent(&coreEvent{Type: "error", Raw: raw})
	if !s.busy {
		t.Fatal("mid-turn error must NOT clear busy (done/aborted still expected)")
	}
}

func TestAbortTimeoutClearsBusy(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.busy = true
	s.abortGen = 3
	model, cmd := s.Update(abortTimeoutMsg{gen: 3})
	_ = cmd
	out := model.(*session)
	if out.busy {
		t.Fatal("abortTimeoutMsg must clear busy when gen matches")
	}
}

func TestAbortTimeoutIgnoresStaleGen(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.busy = true
	s.abortGen = 5
	_, _ = s.Update(abortTimeoutMsg{gen: 4})
	if !s.busy {
		t.Fatal("stale abortTimeoutMsg must not clear busy")
	}
}

func TestArmAbortTimeoutReturnsTick(t *testing.T) {
	s := initialSession()
	s.busy = true
	before := s.abortGen
	cmd := s.armAbortTimeout()
	if cmd == nil {
		t.Fatal("armAbortTimeout must return a tea.Cmd")
	}
	if s.abortGen != before+1 {
		t.Fatalf("abortGen=%d want %d", s.abortGen, before+1)
	}
	// Do not execute the Cmd — tea.Tick waits abortBusyTimeout (15s).
}

func TestCoreStartErrorClearsBusy(t *testing.T) {
	s := initialSession()
	s.busy = true
	s.coreStartGen = 1
	_, _ = s.Update(coreStartErrorMsg{err: errString("boom"), gen: 1})
	if s.busy {
		t.Fatal("coreStartErrorMsg must clear busy")
	}
	if s.coreLifecycle != coreFailed {
		t.Fatalf("coreLifecycle=%v want coreFailed", s.coreLifecycle)
	}
}

type errString string

func (e errString) Error() string { return string(e) }
