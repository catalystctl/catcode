package main

import (
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

func TestModalPasteOwnedByFilterAndEditor(t *testing.T) {
	s := initialSession()
	s.openCommandPalette()
	if !s.appendModalPaste("model") || s.modal.filter != "model" {
		t.Fatalf("filter paste was not consumed: filter=%q", s.modal.filter)
	}
	s.openValueEditModal(editTargetRemember, "Remember", "note", "before ")
	if !s.appendModalPaste("after") || s.modal.editBuf.Value() != "before after" {
		t.Fatalf("editor paste was not consumed: value=%q", s.modal.editBuf.Value())
	}
}

func TestInvalidValueEditRetainsModalAndText(t *testing.T) {
	s := initialSession()
	s.openValueEditModal(editTargetIdleTimeout, "Idle Timeout", "seconds", "9")
	s.commitValueEdit()
	if s.modal.kind != modalValueEdit || !s.modal.editing {
		t.Fatalf("invalid edit closed modal: kind=%v editing=%v", s.modal.kind, s.modal.editing)
	}
	if s.modal.editBuf.Value() != "9" {
		t.Fatalf("invalid edit lost text: %q", s.modal.editBuf.Value())
	}
	if !strings.Contains(strings.ToLower(s.modal.loadError), "10 seconds") {
		t.Fatalf("missing inline validation error: %q", s.modal.loadError)
	}
}

func TestGoalPlanRequiresExplicitApprovalKey(t *testing.T) {
	s := initialSession()
	s.openGoalPlanReview()
	s.handleGoalPlanKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.modal.kind != modalGoalPlan {
		t.Fatal("bare Enter approved or dismissed the goal plan")
	}
}

func TestGoalPlanReviewScrollsAndShowsStepDetail(t *testing.T) {
	s := initialSession()
	s.width, s.height = 64, 12
	s.goalState = &goalStateSnap{Goal: strings.Repeat("goal ", 20)}
	for i := 0; i < 12; i++ {
		s.goalState.Prompts = append(s.goalState.Prompts, goalPromptSnap{
			StepID: "step", Agent: "worker", Title: "Detailed step", Status: "planned",
			Summary: "Implementation detail that must be reviewed before deployment.",
		})
	}
	s.openGoalPlanReview()
	first := stripANSI(s.renderGoalPlanModal())
	if !strings.Contains(first, "status: planned") || !strings.Contains(first, "1–") {
		t.Fatalf("plan lacks detail or scroll position:\n%s", first)
	}
	if got := lipgloss.Height(first); got > s.height {
		t.Fatalf("plan modal height=%d exceeds terminal=%d", got, s.height)
	}
	s.handleGoalPlanKey(tea.KeyPressMsg{Code: tea.KeyDown})
	if s.modal.scroll == 0 {
		t.Fatal("goal review did not scroll")
	}
}

func TestDisabledAndConflictingKeybindsAreTruthful(t *testing.T) {
	s := initialSession()
	s.width, s.height = 100, 28
	s.openKeybindsModal()
	s.keybinds["quit"] = ""
	view := stripANSI(s.renderKeybindsModal())
	if !strings.Contains(view, "quit") || !strings.Contains(view, "—") {
		t.Fatalf("disabled binding did not render as disabled:\n%s", view)
	}
	// quit and toggle_reasoning share the Global group.
	s.keybinds["quit"] = "ctrl+t"
	s.keybinds["toggle_reasoning"] = "ctrl+t"
	if got := s.keybindConflicts(0); len(got) != 1 || got[0] != "toggle_reasoning" {
		t.Fatalf("same-group conflict not reported: %v", got)
	}
}
