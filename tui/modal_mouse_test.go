package main

import (
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

func TestModalMouseDragSelectsAndCopiesVisibleText(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.coreLifecycle = coreReady
	s.width, s.height = 90, 30
	s.openHelp()
	s.layout()
	_ = s.View() // establishes the exact placed overlay used for hit-testing

	needle := "Help"
	y, x := -1, -1
	for i, line := range s.modalPlain {
		if col := strings.Index(line, needle); col >= 0 {
			y, x = i, lipgloss.Width(line[:col])
			break
		}
	}
	if y < 0 {
		t.Fatalf("help modal does not contain %q", needle)
	}
	s.handleTranscriptMouseClick(tea.MouseClickMsg{X: x, Y: y, Button: tea.MouseLeft})
	s.handleTranscriptMouseMotion(tea.MouseMotionMsg{X: x + len(needle) - 1, Y: y, Button: tea.MouseLeft})
	cmd := s.handleTranscriptMouseRelease(tea.MouseReleaseMsg{X: x + len(needle) - 1, Y: y, Button: tea.MouseLeft})
	if cmd == nil {
		t.Fatal("modal drag release should issue clipboard commands")
	}
	if got := s.selectedModalText(); got != needle {
		t.Fatalf("modal selection = %q, want %q", got, needle)
	}
	styled := s.renderModalSelection(s.renderModalOverlay("base"))
	if stripANSI(styled) != stripANSI(s.renderModalOverlay("base")) {
		t.Fatal("modal selection styling changed visible text")
	}
}

func TestModalMouseWheelScrollsReportWithoutMovingTranscript(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 12
	s.openHelp()
	transcriptOffset := s.viewport.YOffset()

	s.handleMouseWheel(tea.MouseWheelMsg{Button: tea.MouseWheelDown})
	if s.modal.scroll == 0 {
		t.Fatal("wheel down did not scroll the help modal")
	}
	if got := s.viewport.YOffset(); got != transcriptOffset {
		t.Fatalf("modal wheel moved hidden transcript: got %d want %d", got, transcriptOffset)
	}
	s.handleMouseWheel(tea.MouseWheelMsg{Button: tea.MouseWheelUp})
	if s.modal.scroll != 0 {
		t.Fatalf("wheel up did not return report to top: scroll=%d", s.modal.scroll)
	}
}

func TestModalMouseWheelNavigatesHandAndCharmLists(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 90, 30

	s.openSettings()
	s.handleMouseWheel(tea.MouseWheelMsg{Button: tea.MouseWheelDown})
	if s.modal.cursor == 0 {
		t.Fatal("wheel did not move hand-rendered settings list")
	}

	s.openCommandPalette()
	before, ok := s.modal.pickerList.SelectedItem().(catalogItem)
	if !ok {
		t.Fatal("command picker has no selected item")
	}
	s.handleMouseWheel(tea.MouseWheelMsg{Button: tea.MouseWheelDown})
	after, ok := s.modal.pickerList.SelectedItem().(catalogItem)
	if !ok || after.abs == before.abs {
		t.Fatalf("wheel did not move Charm picker: before=%d after=%d", before.abs, after.abs)
	}
}

func TestModalMouseWheelIgnoredDuringKeyCapture(t *testing.T) {
	s := initialSession()
	s.openKeybindsModal()
	s.modal.editing = true
	before := s.modal.cursor
	s.handleMouseWheel(tea.MouseWheelMsg{Button: tea.MouseWheelDown})
	if s.modal.cursor != before || !s.modal.editing {
		t.Fatal("wheel changed keybind capture state")
	}
}
