package main

import (
	"testing"

	tea "charm.land/bubbletea/v2"
)

// clickLineForCharmItem returns the screen row of the charm-picker item at
// visible index idx, using the render-recorded geometry.
func clickLineForCharmItem(s *session, idx int) int {
	perPage := s.modal.pickerList.Paginator.PerPage
	row := idx - s.modal.pickerList.Paginator.Page*perPage
	return s.modalBoxTop + s.modalPickerFirst + row*pickerItemPitch
}

// TestModalClickActivatesCharmPickerItem: press moves the highlight, release
// on the same row activates — the palette/models/sessions pickers must behave
// like the menus users expect (the transcript already trains "click rows").
func TestModalClickActivatesCharmPickerItem(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	s := initialSession()
	s.ready = true
	s.authed = true
	s.width, s.height = 100, 32
	s.models = []modelInfo{{ID: "m1"}, {ID: "m2"}, {ID: "m3"}}
	s.modelIdx = 0
	s.openModelPicker()
	s.layout()
	_ = s.View() // populate modalBoxTop / modalPickerFirst / modalPickerRows

	if s.modalPickerRows < 2 {
		t.Fatalf("precondition: need ≥2 clickable rows, got %d", s.modalPickerRows)
	}
	x, y := s.modalBoxLeft+5, clickLineForCharmItem(s, 1)
	s.handleModalMouseClick(tea.MouseClickMsg{X: x, Y: y, Button: tea.MouseLeft})
	if s.modalPressItem != 1 {
		t.Fatalf("press should record item 1, got %d", s.modalPressItem)
	}
	if it, ok := s.modal.pickerList.SelectedItem().(catalogItem); !ok || it.title != "m2" {
		t.Fatalf("press should move highlight to m2, got %+v", s.modal.pickerList.SelectedItem())
	}
	s.handleModalMouseRelease(tea.MouseReleaseMsg{X: x, Y: y, Button: tea.MouseLeft})
	if s.modelIdx != 1 {
		t.Fatalf("click on m2 should select it: modelIdx=%d", s.modelIdx)
	}
	if s.modal.kind != modalNone {
		t.Fatalf("modal should close after activation, got %v", s.modal.kind)
	}
}

// TestModalDragDoesNotActivate: a press-drag-release across rows is a
// copy-selection, not a click — nothing must activate.
func TestModalDragDoesNotActivate(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	s := initialSession()
	s.ready = true
	s.authed = true
	s.width, s.height = 100, 32
	s.models = []modelInfo{{ID: "m1"}, {ID: "m2"}, {ID: "m3"}}
	s.modelIdx = 0
	s.openModelPicker()
	s.layout()
	_ = s.View()

	x := s.modalBoxLeft + 5
	y0, y1 := clickLineForCharmItem(s, 0), clickLineForCharmItem(s, 1)
	s.handleModalMouseClick(tea.MouseClickMsg{X: x, Y: y0, Button: tea.MouseLeft})
	s.handleModalMouseMotion(tea.MouseMotionMsg{X: x, Y: y1, Button: tea.MouseLeft})
	s.handleModalMouseRelease(tea.MouseReleaseMsg{X: x, Y: y1, Button: tea.MouseLeft})
	if s.modelIdx != 0 {
		t.Fatalf("drag must not activate: modelIdx=%d", s.modelIdx)
	}
	if s.modal.kind != modalModels {
		t.Fatalf("modal should stay open after drag, got %v", s.modal.kind)
	}
}

// TestModalClickActivatesCustomListItem: custom renderListModal modals
// (reasoning, plugins, memories, …) get the same click-to-activate behavior
// via the render-recorded row map + synthesized Enter.
func TestModalClickActivatesCustomListItem(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	s := initialSession()
	s.ready = true
	s.authed = true
	s.width, s.height = 100, 32
	items := s.reasoningItems()
	if len(items) < 2 {
		t.Skip("need ≥2 reasoning levels")
	}
	s.settings.ReasoningEffort = items[0].label
	s.openReasoningPicker()
	s.layout()
	_ = s.View()

	// Locate the recorded box-relative line for filtered index 1.
	line := -1
	for boxRel, vi := range s.modalItemRow {
		if vi == 1 {
			line = s.modalBoxTop + boxRel
			break
		}
	}
	if line < 0 {
		t.Fatal("no recorded clickable row for item 1")
	}
	x := s.modalBoxLeft + 5
	s.handleModalMouseClick(tea.MouseClickMsg{X: x, Y: line, Button: tea.MouseLeft})
	if s.modal.cursor != 1 {
		t.Fatalf("press should move cursor to 1, got %d", s.modal.cursor)
	}
	s.handleModalMouseRelease(tea.MouseReleaseMsg{X: x, Y: line, Button: tea.MouseLeft})
	if s.settings.ReasoningEffort != items[1].label {
		t.Fatalf("click should set effort %q, got %q", items[1].label, s.settings.ReasoningEffort)
	}
}
