package main

import (
	"encoding/json"
	"fmt"
	"strings"
	"testing"
	"time"

	"charm.land/lipgloss/v2"
)

func TestFullViewFitsTinyTerminals(t *testing.T) {
	for _, size := range []struct{ w, h int }{{20, 8}, {30, 10}} {
		t.Run(fmt.Sprintf("%dx%d", size.w, size.h), func(t *testing.T) {
			s := initialSession()
			s.ready = true
			s.width, s.height = size.w, size.h
			s.authed = true
			s.models = []modelInfo{{ID: "provider/extremely-long-model-identifier", ContextWindow: 128000}}
			s.modelIdx = 0
			s.contextTokens = 96000
			s.layout()

			view := s.View().Content
			assertFitsViewport(t, "full view", view, size.w, size.h)
			plain := stripANSI(view)
			if !strings.Contains(plain, "Catalyst") {
				t.Fatalf("compact header lost product identity:\n%s", plain)
			}
			if !strings.Contains(plain, "Chat with") {
				t.Fatalf("composer was clipped from tiny full view:\n%s", plain)
			}
			for i, line := range strings.Split(view, "\n") {
				if got := lipgloss.Width(line); got > size.w {
					t.Fatalf("line %d width=%d exceeds %d:\n%s", i, got, size.w, plain)
				}
			}
		})
	}
}

func TestViewContainmentDoesNotPaintRowRemainders(t *testing.T) {
	got := constrainViewContent("short\ntext", 80, 24)
	if strings.Contains(got, "\x1b[48;") || strings.Contains(got, "\x1b[48:") {
		t.Fatalf("containment introduced a background SGR that can bleed across rows: %q", got)
	}
}

func TestCompactPanelsNeverForceTerminalWidth(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 20, 12
	s.todos = []map[string]json.RawMessage{{
		"subject": json.RawMessage(`"a very long task description that cannot fit"`),
		"status":  json.RawMessage(`"pending"`),
	}}
	s.maxTaskRows = 1
	s.subProgress = []*subProgressEntry{{
		agent: "long-running-agent-name", started: time.Now(), curTool: "read_file", toolRunning: true, toolStart: time.Now(),
	}}
	assertFitsViewport(t, "todo", s.renderTodoPanel(), s.width, s.height)
	assertFitsViewport(t, "subagents", s.renderActiveTasks(s.width), s.width, s.height)
}

func TestActivityShelfPrioritizesSubagentsAndScrollsIntoTasks(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.activityExpanded = true
	for i := 0; i < 8; i++ {
		s.subProgress = append(s.subProgress, &subProgressEntry{
			agent: fmt.Sprintf("agent-%d", i), started: time.Now(),
			curTool: "read_file", toolStart: time.Now(), toolRunning: true,
		})
	}
	for i := 0; i < 3; i++ {
		s.todos = append(s.todos, map[string]json.RawMessage{
			"subject": json.RawMessage(fmt.Sprintf("%q", fmt.Sprintf("task-%d", i))),
			"status":  json.RawMessage(`"pending"`),
		})
	}
	s.layout()
	top := stripANSI(s.renderActivityShelf())
	if !strings.Contains(top, "Subagents") || !strings.Contains(top, "agent-0") {
		t.Fatalf("activity shelf did not prioritize subagents:\n%s", top)
	}
	_, _ = s.handleKey(keyMsg("pgdown"))
	_, _ = s.handleKey(keyMsg("pgdown"))
	bottom := stripANSI(s.renderActivityShelf())
	if !strings.Contains(bottom, "Tasks") || !strings.Contains(bottom, "task-2") {
		t.Fatalf("activity shelf could not scroll through tasks:\n%s", bottom)
	}
}

func TestApplyThemePreservesAuthoredPalette(t *testing.T) {
	original := activeTheme
	t.Cleanup(func() { applyTheme(original) })
	for _, th := range themes {
		applyTheme(th)
		if c.bg != th.bg || c.fg != th.fg || c.dim != th.dim || c.muted != th.muted || c.accent != th.accent {
			t.Errorf("theme %q palette was rewritten: got bg=%s fg=%s dim=%s muted=%s accent=%s", th.name, c.bg, c.fg, c.dim, c.muted, c.accent)
		}
	}
}

func TestEveryThemeDerivesAccessibleSemanticColors(t *testing.T) {
	original := activeTheme
	t.Cleanup(func() { applyTheme(original) })
	for _, th := range themes {
		applyTheme(th)
		if got := colorContrast(c.secondary, c.bg); got < 4.5 {
			t.Errorf("theme %q secondary text contrast=%.2f, want >= 4.5", th.name, got)
		}
		if got := colorContrast(c.decor, c.bg); got < 3.0 {
			t.Errorf("theme %q boundary contrast=%.2f, want >= 3.0", th.name, got)
		}
	}
}

func TestComposerUsesEveryThemePaletteInsteadOfTerminalGrey(t *testing.T) {
	if colorsDisabled() {
		t.Skip("color styles are intentionally disabled by NO_COLOR")
	}
	original := activeTheme
	t.Cleanup(func() { applyTheme(original) })
	for _, th := range themes {
		applyTheme(th)
		if got, want := composerTextStyle().GetForeground(), lipgloss.Color(c.fg); got != want {
			t.Errorf("theme %q composer text=%v, want theme foreground %v", th.name, got, want)
		}
		cursor := composerCursorStyle()
		if got, want := cursor.GetBackground(), lipgloss.Color(c.accent); got != want {
			t.Errorf("theme %q cursor background=%v, want accent %v", th.name, got, want)
		}
		if got, want := cursor.GetForeground(), lipgloss.Color(c.bg); got != want {
			t.Errorf("theme %q cursor foreground=%v, want background %v", th.name, got, want)
		}
	}
}

func assertFitsViewport(t *testing.T, name, view string, width, height int) {
	t.Helper()
	if got := lipgloss.Width(view); got > width {
		t.Errorf("%s width=%d exceeds viewport=%d\n%s", name, got, width, stripANSI(view))
	}
	if got := lipgloss.Height(view); got > height {
		t.Errorf("%s height=%d exceeds viewport=%d\n%s", name, got, height, stripANSI(view))
	}
}

func TestCompactOverlaysFitViewport(t *testing.T) {
	for _, size := range []struct{ w, h int }{{20, 8}, {30, 12}, {40, 10}} {
		s := initialSession()
		s.ready = true
		s.width, s.height = size.w, size.h

		s.openCommandPalette()
		assertFitsViewport(t, "command", s.renderModalBody(), size.w, size.h)

		s.pendingSudo = newSudoPrompt("r", "sudo echo 界界界界界界界")
		assertFitsViewport(t, "sudo", s.renderSudoBox(), size.w, size.h)
	}
}

func TestGoalAndAskKeepFocusedControlVisible(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 30, 8
	s.openGoalModal("ship the release")
	s.goalDraft.advanced = true
	s.goalDraft.field = goalFieldStart
	goal := stripANSI(s.renderGoalModal())
	assertFitsViewport(t, "goal", s.renderGoalModal(), s.width, s.height)
	if !strings.Contains(goal, "Start goal") {
		t.Fatalf("focused goal action was clipped:\n%s", goal)
	}

	raw := json.RawMessage(`[
		{"id":"q0","prompt":"Q0","type":"text"},
		{"id":"q1","prompt":"Q1","type":"text"},
		{"id":"q2","prompt":"Q2","type":"text"},
		{"id":"q3","prompt":"Q3","type":"text"}
	]`)
	s.pendingAsk = parseAskRequest("ask", raw)
	s.pendingAsk.focusIdx = 3
	s.pendingAsk.focusInput()
	ask := stripANSI(s.renderAskBox())
	assertFitsViewport(t, "ask", s.renderAskBox(), s.width, s.height)
	if !strings.Contains(ask, "Q3") {
		t.Fatalf("focused ask question was clipped:\n%s", ask)
	}
}

func TestGroupedListBudgetsHeadingRows(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 30, 9
	s.openCommandPalette()
	box := s.renderModalBody()
	assertFitsViewport(t, "grouped command list", box, s.width, s.height)
}

func TestIdentityFirstRowsAndUnicodeCells(t *testing.T) {
	row := stripANSI(fitIdentityListRow("  ", "/model", "a deliberately verbose explanation", 2, 14))
	if !strings.Contains(row, "/model") {
		t.Fatalf("identity disappeared before description: %q", row)
	}
	if got := lipgloss.Width(row); got > 14 {
		t.Fatalf("identity row width=%d: %q", got, row)
	}

	wrapped := wrapPlain("界界界", 4)
	for _, line := range strings.Split(wrapped, "\n") {
		if got := lipgloss.Width(line); got > 4 {
			t.Fatalf("wide-character wrap overflowed: width=%d line=%q", got, line)
		}
	}
	// The family emoji is one grapheme cluster and must not be cut apart.
	family := "👨‍👩‍👧‍👦"
	got := truncate(family+"abcdef", 3)
	if !strings.HasPrefix(got, family) || lipgloss.Width(got) > 3 {
		t.Fatalf("grapheme-aware truncate=%q width=%d", got, lipgloss.Width(got))
	}
}
