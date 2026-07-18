package main

import (
	"strings"
	"time"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

// modalPointAt maps a terminal cell directly into the last rendered modal
// overlay. The overlay is a full-screen placed canvas, so unlike transcript
// selection there is no viewport offset to translate.
func (s *session) modalPointAt(x, y int) (transcriptPoint, bool) {
	if s.modal.kind == modalNone || len(s.modalPlain) == 0 || y < 0 || y >= len(s.modalPlain) {
		return transcriptPoint{}, false
	}
	width := lipgloss.Width(s.modalPlain[y])
	if width == 0 {
		return transcriptPoint{line: y}, true
	}
	return transcriptPoint{line: y, col: min(max(0, x), width-1)}, true
}

func selectionBounds(selection transcriptSelection, lines []string) (start, end transcriptPoint, ok bool) {
	if !selection.dragged || len(lines) == 0 {
		return transcriptPoint{}, transcriptPoint{}, false
	}
	start, end = selection.anchor, selection.head
	if pointBefore(end, start) {
		start, end = end, start
	}
	start.line = min(max(0, start.line), len(lines)-1)
	end.line = min(max(0, end.line), len(lines)-1)
	start.col, _ = cellBoundary(lines[start.line], start.col)
	_, end.col = cellBoundary(lines[end.line], end.col)
	if start.line == end.line && start.col >= end.col {
		return transcriptPoint{}, transcriptPoint{}, false
	}
	return start, end, true
}

func selectedText(selection transcriptSelection, lines []string) string {
	start, end, ok := selectionBounds(selection, lines)
	if !ok {
		return ""
	}
	selected := make([]string, 0, end.line-start.line+1)
	for line := start.line; line <= end.line; line++ {
		switch {
		case line == start.line && line == end.line:
			selected = append(selected, displayCellSlice(lines[line], start.col, end.col))
		case line == start.line:
			selected = append(selected, displayCellSlice(lines[line], start.col, lipgloss.Width(lines[line])))
		case line == end.line:
			selected = append(selected, displayCellSlice(lines[line], 0, end.col))
		default:
			selected = append(selected, lines[line])
		}
	}
	return strings.Join(selected, "\n")
}

func (s *session) selectedModalText() string {
	if s.modalSelectionKind != s.modal.kind {
		return ""
	}
	return selectedText(s.modalSelection, s.modalPlain)
}

func (s *session) renderModalSelection(view string) string {
	if view == "" || s.modalSelectionKind != s.modal.kind {
		return view
	}
	start, end, ok := selectionBounds(s.modalSelection, s.modalPlain)
	if !ok {
		return view
	}
	rows := strings.Split(view, "\n")
	for line := start.line; line <= end.line && line < len(rows); line++ {
		lo, hi := 0, lipgloss.Width(s.modalPlain[line])
		if line == start.line {
			lo = start.col
		}
		if line == end.line {
			hi = end.col
		}
		if hi > lo {
			rows[line] = lipgloss.StyleRanges(rows[line], lipgloss.NewRange(lo, hi, selectionStyle))
		}
	}
	return strings.Join(rows, "\n")
}

func (s *session) handleModalMouseClick(msg tea.MouseClickMsg) tea.Cmd {
	if msg.Button != tea.MouseLeft {
		return nil
	}
	point, ok := s.modalPointAt(msg.X, msg.Y)
	if !ok {
		return nil
	}
	s.modalSelection = transcriptSelection{active: true, anchor: point, head: point}
	s.modalSelectionKind = s.modal.kind
	s.selectionFrameGeneration++
	s.selectionPending = false
	s.selectionFrameScheduled = false
	s.selectionLastFrame = time.Time{}
	return nil
}

func (s *session) handleModalMouseMotion(msg tea.MouseMotionMsg) tea.Cmd {
	if !s.modalSelection.active || s.modalSelectionKind != s.modal.kind {
		s.reuseLastView = true
		return nil
	}
	s.selectionMotion = msg
	s.selectionPending = true
	now := time.Now()
	if s.selectionLastFrame.IsZero() || now.Sub(s.selectionLastFrame) >= selectionFrameInterval {
		s.selectionPending = false
		s.selectionLastFrame = now
		s.applyModalMouseMotion(msg)
		return nil
	}

	s.reuseLastView = true
	if s.selectionFrameScheduled {
		return nil
	}
	s.selectionFrameScheduled = true
	delay := selectionFrameInterval - now.Sub(s.selectionLastFrame)
	if delay < 0 {
		delay = 0
	}
	generation := s.selectionFrameGeneration
	return tea.Tick(delay, func(time.Time) tea.Msg {
		return selectionFrameMsg{generation: generation, modal: true}
	})
}

func (s *session) applyModalMouseMotion(msg tea.MouseMotionMsg) {
	point, ok := s.modalPointAt(msg.X, msg.Y)
	if !ok {
		return
	}
	if point != s.modalSelection.anchor {
		s.modalSelection.dragged = true
	}
	s.modalSelection.head = point
}

func (s *session) handleModalSelectionFrame(msg selectionFrameMsg) tea.Cmd {
	if msg.generation != s.selectionFrameGeneration ||
		!s.modalSelection.active || s.modalSelectionKind != s.modal.kind {
		s.reuseLastView = true
		return nil
	}
	s.selectionFrameScheduled = false
	if !s.selectionPending {
		s.reuseLastView = true
		return nil
	}
	motion := s.selectionMotion
	s.selectionPending = false
	s.selectionLastFrame = time.Now()
	s.applyModalMouseMotion(motion)
	return nil
}

func (s *session) handleModalMouseRelease(msg tea.MouseReleaseMsg) tea.Cmd {
	if !s.modalSelection.active || s.modalSelectionKind != s.modal.kind {
		s.reuseLastView = true
		return nil
	}
	s.selectionFrameGeneration++
	s.selectionPending = false
	s.selectionFrameScheduled = false
	if point, ok := s.modalPointAt(msg.X, msg.Y); ok {
		if point != s.modalSelection.anchor {
			s.modalSelection.dragged = true
		}
		s.modalSelection.head = point
	}
	s.modalSelection.active = false
	text := s.selectedModalText()
	if text == "" {
		return nil
	}
	s.setToast(toastSuccess, "copied modal selection")
	return tea.Batch(tea.SetClipboard(text), tea.SetPrimaryClipboard(text))
}

// handleModalMouseWheel reuses each modal's keyboard navigation so scrolling
// stays consistent with arrows: reports move their window, lists move the
// highlighted row, and focus-windowed forms move to the next visible field.
func (s *session) handleModalMouseWheel(msg tea.MouseWheelMsg) tea.Cmd {
	if s.modal.kind == modalNone || s.modal.kind == modalConfirm || s.modal.editing {
		// Wheel input must never alter a confirmation choice or get captured as
		// a key binding while an editor/capture field owns keyboard input.
		return nil
	}
	var code rune
	switch msg.Button {
	case tea.MouseWheelUp:
		code = tea.KeyUp
	case tea.MouseWheelDown:
		code = tea.KeyDown
	default:
		return nil
	}
	steps := max(1, s.viewport.MouseWheelDelta)
	cmds := make([]tea.Cmd, 0, steps)
	for range steps {
		_, cmd := s.handleModalKey(tea.KeyPressMsg{Code: code})
		if cmd != nil {
			cmds = append(cmds, cmd)
		}
		if s.modal.kind == modalNone {
			break
		}
	}
	if len(cmds) == 0 {
		return nil
	}
	return tea.Batch(cmds...)
}
