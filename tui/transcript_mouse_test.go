package main

import (
	"strings"
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

func mouseTranscriptSession(t *testing.T) (*session, *block, *block, *block) {
	t.Helper()
	s := initialSession()
	s.ready = true
	s.coreLifecycle = coreReady
	s.width, s.height = 90, 36
	s.models = []modelInfo{{ID: "test-model"}}
	s.modelIdx = 0
	s.settings.path = t.TempDir() + "/settings.json"
	s.layout()

	user := s.push(blkUser)
	user.appendText("Select this transcript text")
	reasoning := s.push(blkThinking)
	reasoning.appendText("Inspect the parser carefully before changing it.")
	read := s.push(blkTool)
	read.name = "read_file"
	read.args = `{"path":"parser.go"}`
	read.output = "package parser\nfunc parse() {}"
	read.hasOk, read.ok, read.dur = true, true, 20*time.Millisecond
	edit := s.push(blkTool)
	edit.name = "edit"
	edit.args = `{"path":"parser.go","edits":[{"old_text":"parse","new_text":"Parse"}]}`
	edit.output = "updated parser.go"
	edit.hasOk, edit.ok, edit.dur = true, true, 30*time.Millisecond
	answer := s.push(blkAssistant)
	answer.appendText("The parser is updated.")
	s.cur = nil
	s.invalidateAll()
	s.refresh()
	return s, reasoning, read, edit
}

func transcriptMouseY(s *session, line int) int {
	return s.transcriptViewportTop() + line - s.viewport.YOffset()
}

func transcriptNeedleCell(t *testing.T, s *session, line int, needle string) int {
	t.Helper()
	text := s.transcriptPlainLines()[line]
	byteOffset := strings.Index(text, needle)
	if byteOffset < 0 {
		t.Fatalf("transcript line %q does not contain %q", text, needle)
	}
	return lipgloss.Width(text[:byteOffset])
}

func clickTranscriptLine(s *session, line, x int) tea.Cmd {
	y := transcriptMouseY(s, line)
	s.handleTranscriptMouseClick(tea.MouseClickMsg{X: x, Y: y, Button: tea.MouseLeft})
	return s.handleTranscriptMouseRelease(tea.MouseReleaseMsg{X: x, Y: y, Button: tea.MouseLeft})
}

func TestTranscriptMouseDragSelectsHighlightsAndCopies(t *testing.T) {
	s, _, _, _ := mouseTranscriptSession(t)
	// User content begins one line below its role header.
	line := s.blocks[0].renderStart + 1
	y := transcriptMouseY(s, line)
	start := transcriptNeedleCell(t, s, line, "Select")
	s.handleTranscriptMouseClick(tea.MouseClickMsg{X: start, Y: y, Button: tea.MouseLeft})
	s.handleTranscriptMouseMotion(tea.MouseMotionMsg{X: start + 5, Y: y, Button: tea.MouseLeft})
	cmd := s.handleTranscriptMouseRelease(tea.MouseReleaseMsg{X: start + 5, Y: y, Button: tea.MouseLeft})

	if cmd == nil {
		t.Fatal("drag release should return clipboard commands")
	}
	if got := s.selectedTranscriptText(); got != "Select" {
		t.Fatalf("selected text = %q, want %q", got, "Select")
	}
	if !s.selection.dragged || s.selection.active {
		t.Fatalf("selection state after release = %+v", s.selection)
	}
	visible := s.renderVisibleTranscriptSelection(s.viewport.View())
	if visible == s.viewport.View() {
		t.Fatal("visible viewport should contain selection styling after a drag")
	}
	if stripANSI(visible) != stripANSI(s.viewport.View()) {
		t.Fatal("selection styling must not alter transcript text")
	}
}

func TestDisplayCellSliceKeepsWideGraphemesWhole(t *testing.T) {
	const text = "A界🙂B"
	if got := displayCellSlice(text, 1, 3); got != "界" {
		t.Fatalf("wide CJK slice = %q, want 界", got)
	}
	if got := displayCellSlice(text, 3, 5); got != "🙂" {
		t.Fatalf("emoji slice = %q, want 🙂", got)
	}
	if start, end := cellBoundary(text, 2); start != 1 || end != 3 {
		t.Fatalf("second cell of wide glyph snapped to %d-%d, want 1-3", start, end)
	}
}

func TestTranscriptMouseClickTogglesReasoningAndToolDetails(t *testing.T) {
	s, reasoning, read, _ := mouseTranscriptSession(t)
	if !reasoning.collapsed {
		t.Fatal("reasoning should start collapsed")
	}
	if cmd := clickTranscriptLine(s, reasoning.renderStart, 2); cmd != nil {
		t.Fatal("stationary disclosure click should not issue clipboard command")
	}
	if reasoning.collapsed {
		t.Fatal("reasoning header click should expand only that block")
	}

	if read.expanded {
		t.Fatal("tool should start compact")
	}
	clickTranscriptLine(s, read.renderStart, 4)
	if !read.expanded {
		t.Fatal("tool-row click should reveal full details")
	}
	clickTranscriptLine(s, read.renderStart, 4)
	if read.expanded {
		t.Fatal("second tool-row click should collapse details")
	}
}

func TestTranscriptMouseDragOnDisclosureDoesNotToggleIt(t *testing.T) {
	s, reasoning, _, _ := mouseTranscriptSession(t)
	y := transcriptMouseY(s, reasoning.renderStart)
	s.handleTranscriptMouseClick(tea.MouseClickMsg{X: 0, Y: y, Button: tea.MouseLeft})
	s.handleTranscriptMouseMotion(tea.MouseMotionMsg{X: 9, Y: y, Button: tea.MouseLeft})
	cmd := s.handleTranscriptMouseRelease(tea.MouseReleaseMsg{X: 9, Y: y, Button: tea.MouseLeft})
	if cmd == nil {
		t.Fatal("dragging a disclosure row should copy the selection")
	}
	if !reasoning.collapsed {
		t.Fatal("dragging a disclosure row must not toggle it")
	}
	if !strings.Contains(s.selectedTranscriptText(), "reason") {
		t.Fatalf("unexpected selected header text %q", s.selectedTranscriptText())
	}
}

func TestTranscriptMouseMotionCoalescesToLatestFrame(t *testing.T) {
	s, _, _, _ := mouseTranscriptSession(t)
	line := s.blocks[0].renderStart + 1
	y := transcriptMouseY(s, line)
	s.handleTranscriptMouseClick(tea.MouseClickMsg{X: 3, Y: y, Button: tea.MouseLeft})
	s.handleTranscriptMouseMotion(tea.MouseMotionMsg{X: 5, Y: y, Button: tea.MouseLeft})
	first := s.selection.head

	// Force this event inside the current frame instead of relying on test
	// execution speed. It should be retained without changing painted state.
	s.selectionLastFrame = time.Now()
	cmd := s.handleTranscriptMouseMotion(tea.MouseMotionMsg{X: 8, Y: y, Button: tea.MouseLeft})
	if cmd == nil {
		t.Fatal("first coalesced motion should schedule a selection frame")
	}
	if s.selection.head != first {
		t.Fatalf("coalesced motion changed painted head: got %+v want %+v", s.selection.head, first)
	}
	if !s.selectionPending || !s.reuseLastView {
		t.Fatalf("coalesced state pending=%v reuse=%v", s.selectionPending, s.reuseLastView)
	}

	s.handleTranscriptSelectionFrame(selectionFrameMsg{generation: s.selectionFrameGeneration})
	if s.selection.head.col != 8 {
		t.Fatalf("selection frame did not apply latest pointer: head=%+v", s.selection.head)
	}
}

func TestTranscriptMouseReleaseUsesFinalCoalescedPosition(t *testing.T) {
	s, _, _, _ := mouseTranscriptSession(t)
	line := s.blocks[0].renderStart + 1
	y := transcriptMouseY(s, line)
	start := transcriptNeedleCell(t, s, line, "Select")
	s.handleTranscriptMouseClick(tea.MouseClickMsg{X: start, Y: y, Button: tea.MouseLeft})
	s.handleTranscriptMouseMotion(tea.MouseMotionMsg{X: start + 2, Y: y, Button: tea.MouseLeft})
	s.selectionLastFrame = time.Now()
	s.handleTranscriptMouseMotion(tea.MouseMotionMsg{X: start + 5, Y: y, Button: tea.MouseLeft})

	cmd := s.handleTranscriptMouseRelease(tea.MouseReleaseMsg{X: start + 8, Y: y, Button: tea.MouseLeft})
	if cmd == nil {
		t.Fatal("release should copy the final selection")
	}
	if got := s.selectedTranscriptText(); got != "Select th" {
		t.Fatalf("selected text = %q, want %q", got, "Select th")
	}
	if s.selectionPending || s.selectionFrameScheduled {
		t.Fatalf("release left pending selection work: pending=%v scheduled=%v", s.selectionPending, s.selectionFrameScheduled)
	}
}

func TestActivityHeadingClickTogglesWholeGroup(t *testing.T) {
	s, _, read, edit := mouseTranscriptSession(t)
	headingLine := read.renderStart - 1
	clickTranscriptLine(s, headingLine, 2)
	if !read.expanded || !edit.expanded {
		t.Fatalf("activity heading should expand all calls: read=%v edit=%v", read.expanded, edit.expanded)
	}
	// The group heading remains immediately before the first call after reflow.
	clickTranscriptLine(s, read.renderStart-1, 2)
	if read.expanded || edit.expanded {
		t.Fatalf("second activity-heading click should collapse all calls: read=%v edit=%v", read.expanded, edit.expanded)
	}
}

func TestMouseModeAndWheelAreAlwaysOn(t *testing.T) {
	s, _, _, _ := mouseTranscriptSession(t)
	for i := 0; i < 40; i++ {
		b := s.push(blkUser)
		b.appendText("extra transcript line")
	}
	s.cur = nil
	s.invalidateAll()
	s.refresh()
	view := s.View()
	if view.MouseMode != tea.MouseModeCellMotion {
		t.Fatalf("mouse mode = %v, want cell motion", view.MouseMode)
	}
	before := s.viewport.YOffset()
	s.handleMouseWheel(tea.MouseWheelMsg{Button: tea.MouseWheelUp})
	if s.viewport.YOffset() >= before {
		t.Fatalf("wheel should scroll regardless of legacy setting: before=%d after=%d", before, s.viewport.YOffset())
	}
}
