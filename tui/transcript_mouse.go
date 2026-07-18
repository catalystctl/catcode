package main

import (
	"strings"
	"time"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"
	"github.com/rivo/uniseg"
)

// transcriptViewportTop returns the first terminal row owned by the viewport.
// It mirrors the prefix assembled in View; keeping this calculation in one
// place makes mouse coordinates stable when a header banner appears.
func (s *session) transcriptViewportTop() int {
	top := lipgloss.Height(s.renderHeader())
	if b := s.renderCoreFailureBanner(); b != "" {
		top += lipgloss.Height(b)
	}
	if b := s.renderUpdateBanner(); b != "" && s.height >= 10 {
		top += lipgloss.Height(b)
	}
	if b := s.renderOauthBanner(); b != "" {
		top += lipgloss.Height(b)
	}
	return top
}

func (s *session) transcriptPlainLines() []string {
	return s.transcriptPlain
}

func plainTranscriptLines(content string) []string {
	if content == "" {
		return nil
	}
	styled := strings.Split(content, "\n")
	plain := make([]string, len(styled))
	for i := range styled {
		plain[i] = ansi.Strip(styled[i])
	}
	return plain
}

// transcriptPointAt maps a terminal cell into the uncropped transcript. When
// clampOutside is true (dragging), coordinates just outside the viewport are
// pinned to its edge so a selection can continue while auto-scrolling.
func (s *session) transcriptPointAt(x, y int, clampOutside bool) (transcriptPoint, bool) {
	lines := s.transcriptPlainLines()
	if len(lines) == 0 || s.viewport.Height() <= 0 {
		return transcriptPoint{}, false
	}
	top := s.transcriptViewportTop()
	row := y - top
	if !clampOutside && (row < 0 || row >= s.viewport.Height()) {
		return transcriptPoint{}, false
	}
	row = min(max(0, row), s.viewport.Height()-1)
	line := s.viewport.YOffset() + row
	if line >= len(lines) {
		if !clampOutside {
			return transcriptPoint{}, false
		}
		line = len(lines) - 1
	}
	col := max(0, x+s.viewport.XOffset())
	width := lipgloss.Width(lines[line])
	if width == 0 {
		col = 0
	} else {
		col = min(col, width-1)
	}
	return transcriptPoint{line: line, col: col}, true
}

func pointBefore(a, b transcriptPoint) bool {
	return a.line < b.line || (a.line == b.line && a.col < b.col)
}

// cellBoundary returns the display-cell start/end of the grapheme containing
// col. A click on the second cell of a wide glyph therefore selects the whole
// glyph and never creates a half-character clipboard result.
func cellBoundary(text string, col int) (start, end int) {
	used := 0
	g := uniseg.NewGraphemes(text)
	for g.Next() {
		width := max(1, g.Width())
		next := used + width
		if col < next {
			return used, next
		}
		used = next
	}
	return used, used
}

// transcriptSelectionBounds returns an ordered, inclusive-line/exclusive-cell
// range. The mouse anchor/head identify cells, so the later endpoint is moved
// to the end of its grapheme before styling or copying.
func (s *session) transcriptSelectionBounds() (start, end transcriptPoint, ok bool) {
	if !s.selection.dragged {
		return transcriptPoint{}, transcriptPoint{}, false
	}
	lines := s.transcriptPlainLines()
	if len(lines) == 0 {
		return transcriptPoint{}, transcriptPoint{}, false
	}
	start, end = s.selection.anchor, s.selection.head
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

// renderVisibleTranscriptSelection overlays selection styling on the cropped
// viewport output, never on the full transcript. Mouse motion can arrive much
// faster than a terminal frame; making this O(viewport height) prevents large
// selections from becoming progressively more expensive as they grow.
func (s *session) renderVisibleTranscriptSelection(view string) string {
	start, end, ok := s.transcriptSelectionBounds()
	if !ok || view == "" {
		return view
	}
	rows := strings.Split(view, "\n")
	plain := s.transcriptPlainLines()
	first := s.viewport.YOffset()
	xOffset := s.viewport.XOffset()
	for row := range rows {
		line := first + row
		if line < start.line || line > end.line || line < 0 || line >= len(plain) {
			continue
		}
		lo, hi := 0, lipgloss.Width(plain[line])
		if line == start.line {
			lo = start.col
		}
		if line == end.line {
			hi = end.col
		}
		lo = max(0, lo-xOffset)
		hi = max(0, hi-xOffset)
		hi = min(hi, lipgloss.Width(rows[row]))
		if hi > lo {
			rows[row] = lipgloss.StyleRanges(rows[row], lipgloss.NewRange(lo, hi, selectionStyle))
		}
	}
	return strings.Join(rows, "\n")
}

func displayCellSlice(text string, start, end int) string {
	if end <= start || text == "" {
		return ""
	}
	var out strings.Builder
	used := 0
	g := uniseg.NewGraphemes(text)
	for g.Next() {
		width := max(1, g.Width())
		next := used + width
		if next > start && used < end {
			out.WriteString(g.Str())
		}
		if used >= end {
			break
		}
		used = next
	}
	return out.String()
}

func (s *session) selectedTranscriptText() string {
	start, end, ok := s.transcriptSelectionBounds()
	if !ok {
		return ""
	}
	lines := s.transcriptPlainLines()
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
			// The transcript cache is already ANSI-free. Interior lines can be
			// copied directly instead of measuring and walking every grapheme.
			selected = append(selected, lines[line])
		}
	}
	return strings.Join(selected, "\n")
}

func (s *session) mouseOverlayOwnsInput() bool {
	return s.modal.kind != modalNone || s.pendingAsk != nil || s.pendingSudo != nil
}

func (s *session) handleTranscriptMouseClick(msg tea.MouseClickMsg) tea.Cmd {
	if s.modal.kind != modalNone {
		return s.handleModalMouseClick(msg)
	}
	if msg.Button != tea.MouseLeft || s.mouseOverlayOwnsInput() {
		return nil
	}
	point, ok := s.transcriptPointAt(msg.X, msg.Y, false)
	if !ok {
		return nil
	}
	s.selection = transcriptSelection{active: true, anchor: point, head: point}
	s.selectionFrameGeneration++
	s.selectionPending = false
	s.selectionFrameScheduled = false
	s.selectionLastFrame = time.Time{}
	// A single click is dispatched on release, after we know it was not a drag.
	// View paints selection directly, so no transcript rebuild is needed here.
	return nil
}

func (s *session) handleTranscriptMouseMotion(msg tea.MouseMotionMsg) tea.Cmd {
	if s.modal.kind != modalNone {
		return s.handleModalMouseMotion(msg)
	}
	if !s.selection.active || s.mouseOverlayOwnsInput() {
		s.reuseLastView = true
		return nil
	}
	s.selectionMotion = msg
	s.selectionPending = true

	// Paint the first movement immediately. After that, retain only the newest
	// pointer position until the next renderer-sized frame. Bubble Tea invokes
	// View for every input event, even when its renderer cannot flush that fast,
	// so skipped events explicitly reuse the preceding View as well.
	now := time.Now()
	if s.selectionLastFrame.IsZero() || now.Sub(s.selectionLastFrame) >= selectionFrameInterval {
		s.selectionPending = false
		s.selectionLastFrame = now
		s.applyTranscriptMouseMotion(msg)
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
		return selectionFrameMsg{generation: generation}
	})
}

func (s *session) applyTranscriptMouseMotion(msg tea.MouseMotionMsg) {
	top := s.transcriptViewportTop()
	bottom := top + s.viewport.Height() - 1
	if msg.Y <= top && !s.viewport.AtTop() {
		s.follow = false
		s.viewport.ScrollUp(1)
	} else if msg.Y >= bottom && !s.viewport.AtBottom() {
		s.follow = false
		s.viewport.ScrollDown(1)
	}
	point, ok := s.transcriptPointAt(msg.X, msg.Y, true)
	if !ok {
		return
	}
	if point != s.selection.anchor {
		s.selection.dragged = true
	}
	s.selection.head = point
}

func (s *session) handleTranscriptSelectionFrame(msg selectionFrameMsg) tea.Cmd {
	if msg.generation != s.selectionFrameGeneration || !s.selection.active {
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
	s.applyTranscriptMouseMotion(motion)
	return nil
}

func (s *session) handleTranscriptMouseRelease(msg tea.MouseReleaseMsg) tea.Cmd {
	if s.modal.kind != modalNone {
		return s.handleModalMouseRelease(msg)
	}
	if !s.selection.active {
		s.reuseLastView = true
		return nil
	}
	// Release coordinates are authoritative even when the last few motion
	// events were coalesced, so copied text always reaches the pointer.
	s.selectionFrameGeneration++
	s.selectionPending = false
	s.selectionFrameScheduled = false
	if point, ok := s.transcriptPointAt(msg.X, msg.Y, true); ok {
		if point != s.selection.anchor {
			s.selection.dragged = true
		}
		s.selection.head = point
	}
	s.selection.active = false
	if s.selection.dragged {
		text := s.selectedTranscriptText()
		if text == "" {
			return nil
		}
		// Avoid another full Unicode scan here: very large selections already
		// have to be assembled once for the clipboard payload.
		s.setToast(toastSuccess, "copied selection")
		return tea.Batch(tea.SetClipboard(text), tea.SetPrimaryClipboard(text))
	}

	point := s.selection.anchor
	s.selection = transcriptSelection{}
	if s.toggleTranscriptDisclosure(point.line) {
		return nil
	}
	return nil
}

// toggleTranscriptDisclosure handles click-without-drag. Reasoning is local to
// the clicked block (Ctrl+T remains the global toggle); a tool row toggles one
// call, while the activity heading toggles every inspectable call in that run.
func (s *session) toggleTranscriptDisclosure(line int) bool {
	for i, b := range s.blocks {
		if b == nil || b.kind != blkThinking || line != b.renderStart {
			continue
		}
		b.collapsed = !b.collapsed
		s.focusedBlock = i
		s.follow = false
		s.invalidateAll()
		s.refresh()
		return true
	}

	for i := 0; i < len(s.blocks); {
		if !isToolActivityBlock(s.blocks[i]) {
			i++
			continue
		}
		end := toolActivityRunEnd(s.blocks, i)
		firstLine := -1
		for _, b := range s.blocks[i:end] {
			if b != nil && b.renderEnd >= b.renderStart && b.renderStart > 0 {
				firstLine = b.renderStart
				break
			}
		}
		if firstLine > 0 && line == firstLine-1 {
			expand := false
			for _, b := range s.blocks[i:end] {
				if b != nil && !b.sub && !b.expanded {
					expand = true
					break
				}
			}
			for j, b := range s.blocks[i:end] {
				if b == nil || b.sub {
					continue
				}
				b.expanded = expand
				s.focusedBlock = i + j
			}
			s.follow = false
			s.invalidateAll()
			s.refresh()
			return true
		}
		for j, b := range s.blocks[i:end] {
			if b == nil || b.sub || line != b.renderStart {
				continue
			}
			b.expanded = !b.expanded
			s.focusedBlock = i + j
			s.follow = false
			s.invalidateAll()
			s.refresh()
			return true
		}
		i = end
	}
	return false
}
