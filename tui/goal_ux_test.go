package main

import (
	"encoding/json"
	"strings"
	"testing"
)

func blockTexts(s *session) []string {
	out := make([]string, 0, len(s.blocks))
	for _, b := range s.blocks {
		if b == nil {
			continue
		}
		out = append(out, b.text.String())
	}
	return out
}

func hasBlockContaining(s *session, needle string) bool {
	for _, t := range blockTexts(s) {
		if strings.Contains(t, needle) {
			return true
		}
	}
	return false
}

func TestGoalStateRunningToDonePersistsStepCard(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24

	running, _ := json.Marshal(map[string]any{
		"type": "goal_state", "id": "g1", "goal": "ship", "phase": "running",
		"auto_deploy": true, "version": 1,
		"prompts": []map[string]any{
			{"step_id": "1", "agent": "worker", "title": "impl", "status": "running"},
		},
	})
	s.handleCoreEvent(&coreEvent{Type: "goal_state", Raw: running})
	if !s.busy {
		t.Fatal("expected busy during running")
	}
	panel := stripANSI(s.renderGoalProgressPanel(s.width))
	if !strings.Contains(panel, "goal · running") || !strings.Contains(panel, "impl") {
		t.Fatalf("progress panel missing running step:\n%s", panel)
	}

	done, _ := json.Marshal(map[string]any{
		"type": "goal_state", "id": "g1", "goal": "ship", "phase": "running",
		"auto_deploy": true, "version": 2,
		"prompts": []map[string]any{
			{"step_id": "1", "agent": "worker", "title": "impl", "status": "done",
				"summary": "Implemented the feature and verified tests."},
		},
	})
	s.handleCoreEvent(&coreEvent{Type: "goal_state", Raw: done})
	if !hasBlockContaining(s, "Implemented the feature") {
		t.Fatalf("expected lasting step summary card, blocks=%v", blockTexts(s))
	}
	if !hasBlockContaining(s, "goal step done") {
		t.Fatalf("expected step done header, blocks=%v", blockTexts(s))
	}
	// Second identical update must not double-fire.
	n := len(s.blocks)
	s.handleCoreEvent(&coreEvent{Type: "goal_state", Raw: done})
	if len(s.blocks) != n {
		t.Fatalf("duplicate step card; before=%d after=%d", n, len(s.blocks))
	}
}

func TestGoalStepVerdictPersists(t *testing.T) {
	s := initialSession()
	s.ready = true
	raw, _ := json.Marshal(map[string]any{
		"type": "goal_step_verdict", "ok": true, "output": "VERDICT: PASS\nlooks good",
	})
	s.handleCoreEvent(&coreEvent{Type: "goal_step_verdict", Raw: raw})
	if !hasBlockContaining(s, "goal verdict PASS") {
		t.Fatalf("verdict not persisted: %v", blockTexts(s))
	}
	if !hasBlockContaining(s, "looks good") {
		t.Fatalf("verdict output missing: %v", blockTexts(s))
	}
}

func TestGoalPhaseSynthesizingPersists(t *testing.T) {
	s := initialSession()
	s.ready = true
	raw, _ := json.Marshal(map[string]any{
		"type": "goal_phase", "from": "running", "to": "synthesizing",
	})
	s.handleCoreEvent(&coreEvent{Type: "goal_phase", Raw: raw})
	if !hasBlockContaining(s, "writing completion summary") {
		t.Fatalf("synthesizing bridge missing: %v", blockTexts(s))
	}
}

func TestGoalStepCompleteEventPersists(t *testing.T) {
	s := initialSession()
	s.ready = true
	raw, _ := json.Marshal(map[string]any{
		"type": "goal_step_complete",
		"step_id": "2", "title": "verify", "agent": "reviewer",
		"ok": true, "status": "done", "summary": "All checks passed.",
	})
	s.handleCoreEvent(&coreEvent{Type: "goal_step_complete", Raw: raw})
	if !hasBlockContaining(s, "All checks passed") {
		t.Fatalf("goal_step_complete not persisted: %v", blockTexts(s))
	}
}

func TestDoneWithGoalKeepsSubProgress(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.busy = true
	s.goalState = &goalStateSnap{ID: "g1", Phase: "running", AutoDeploy: true}
	s.subProgress = []*subProgressEntry{{
		runID: "r1", agent: "worker",
	}}
	s.handleCoreEvent(&coreEvent{Type: "done", Raw: []byte(`{"type":"done"}`)})
	if len(s.subProgress) != 1 {
		t.Fatalf("goalKeepsBusy done wiped subProgress: %+v", s.subProgress)
	}
	if !s.busy {
		t.Fatal("expected still busy under goalKeepsBusy")
	}
}

func TestGoalDoneClearsBusy(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.busy = true
	s.goalState = &goalStateSnap{
		ID: "g1", Phase: "synthesizing", Goal: "ship",
		Prompts: []goalPromptSnap{{StepID: "1", Title: "a", Status: "done", Summary: "ok"}},
	}
	done, _ := json.Marshal(map[string]any{
		"type": "goal_state", "id": "g1", "goal": "ship", "phase": "done",
		"prompts": []map[string]any{
			{"step_id": "1", "title": "a", "agent": "worker", "status": "done", "summary": "ok"},
		},
	})
	s.handleCoreEvent(&coreEvent{Type: "goal_state", Raw: done})
	if s.busy {
		t.Fatal("expected busy cleared when goal reaches done")
	}
}

func TestGoalPhaseIncludesWaveCounts(t *testing.T) {
	s := initialSession()
	s.ready = true
	raw, _ := json.Marshal(map[string]any{
		"type": "goal_phase", "from": "deploying", "to": "running",
		"message": "wave 2", "wave": 2, "done_count": 1, "step_count": 3,
	})
	s.handleCoreEvent(&coreEvent{Type: "goal_phase", Raw: raw})
	if !hasBlockContaining(s, "wave 2") || !hasBlockContaining(s, "(1/3)") {
		t.Fatalf("expected wave/counts in lasting phase line, got %v", blockTexts(s))
	}
}

func TestGoalEmptySummaryNeverBlank(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.persistGoalStepComplete("1", "empty", "worker", "done", "")
	if !hasBlockContaining(s, "no written summary") {
		t.Fatalf("empty summary should stub, got %v", blockTexts(s))
	}
	if hasBlockContaining(s, "[no result]") {
		t.Fatal("must not show [no result] for goal step cards")
	}
}

func TestFinishBlockEmptyFallback(t *testing.T) {
	b := &block{kind: blkTool, name: "finish", output: ""}
	out := stripANSI(renderFinishBlock(b, 80))
	if !strings.Contains(out, "This turn has finished") && !strings.Contains(out, "finish") {
		// renderFinishBlock puts msg in extra only when !inFlight; set dur.
		b.dur = 1
		b.hasOk = true
		b.ok = true
		out = stripANSI(renderFinishBlock(b, 80))
	}
	b.dur = 1
	b.hasOk = true
	b.ok = true
	b.output = "[no result]"
	out = stripANSI(renderFinishBlock(b, 80))
	if !strings.Contains(out, "This turn has finished") {
		t.Fatalf("finish fallback missing: %q", out)
	}
}

func TestGoalHeaderShowsPhase(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 100, 24
	s.busy = true
	s.goalState = &goalStateSnap{
		ID: "g1", Phase: "running",
		Prompts: []goalPromptSnap{
			{StepID: "1", Title: "a", Status: "done"},
			{StepID: "2", Title: "b", Status: "running"},
		},
	}
	head := stripANSI(s.renderHeader())
	if !strings.Contains(head, "goal · running · 1/2") {
		t.Fatalf("header missing goal progress: %q", head)
	}
}

func TestGoalInfoDeployPersists(t *testing.T) {
	s := initialSession()
	s.ready = true
	raw, _ := json.Marshal(map[string]any{
		"type": "info",
		"message": "Goal deploy complete — writing completion summary…",
	})
	s.handleCoreEvent(&coreEvent{Type: "info", Raw: raw})
	if !hasBlockContaining(s, "Goal deploy complete") {
		t.Fatalf("deploy info should persist: %v", blockTexts(s))
	}
}
