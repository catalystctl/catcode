package main

import "testing"

// TestSkillPaletteInsertsIntoInput: selecting a /skill:<name> entry from the
// command palette must INSERT the token (+ trailing space) into the input box
// instead of dispatching immediately, so the user can append a task message and
// send them as one turn. Press Enter again to run the bare skill.
func TestSkillPaletteInsertsIntoInput(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.skillsList = []skillInfo{{Name: "frontend-design", Description: "design skill"}}
	s.openCommandPalette()

	idx := -1
	for i, it := range s.commandItems() {
		if it.label == "/skill:frontend-design" {
			idx = i
			break
		}
	}
	if idx < 0 {
		t.Fatal("skill entry should appear in the command palette")
	}

	s.runCommandByIndex(idx)

	if got := s.input.Value(); got != "/skill:frontend-design " {
		t.Fatalf("selecting a skill should insert token+space into input; got %q", got)
	}
	if s.modal.kind != modalNone {
		t.Fatalf("palette should close after inserting a skill; modal kind=%v", s.modal.kind)
	}
	if s.busy {
		t.Fatal("selecting a skill should NOT dispatch a turn (no in-flight request)")
	}
}

// TestSkillPaletteBareDispatchesOnEnter: once /skill:<name> is in the input,
// pressing Enter with no extra text runs the bare skill (no task) — the
// input is consumed and a turn is dispatched. This confirms the insert leaves
// the normal typed-invocation path intact.
func TestSkillPaletteBareDispatchesOnEnter(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.authed = true
	s.models = []modelInfo{{ID: "m1", Name: "Model 1"}}
	s.modelIdx = 0
	s.skillsList = []skillInfo{{Name: "frontend-design", Description: "design skill"}}

	// Simulate the palette-insert result, then Enter. handleUserLine owns the
	// input reset in this code path; the bare token dispatches apply_skill.
	s.handleUserLine("/skill:frontend-design ")

	if !s.busy {
		t.Fatal("bare skill on Enter should dispatch a turn (busy should be true)")
	}
}
