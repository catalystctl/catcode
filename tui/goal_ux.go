package main

import (
	"fmt"
	"strconv"
	"strings"

	"charm.land/lipgloss/v2"
)

// goalDisplaySummaryCap is the max chars shown in lasting step-complete
// transcript cards (panel may show a shorter one-line preview).
const goalDisplaySummaryCap = 800

// goalShowsProgressPanel is true while the pinned goal progress panel should
// render. plan_ready only counts when auto-deploy is armed (parity with web).
func goalShowsProgressPanel(phase string, autoDeploy bool) bool {
	switch phase {
	case "deploying", "running", "synthesizing", "done", "failed":
		return true
	case "plan_ready":
		return autoDeploy
	default:
		return false
	}
}

// goalProgressPhaseLabel is the short phase shown in headers/panels.
// plan_ready+autoDeploy renders as "starting" (parity with web).
func goalProgressPhaseLabel(phase string, autoDeploy bool) string {
	if phase == "plan_ready" && autoDeploy {
		return "starting"
	}
	return phase
}

func goalTerminalStatus(status string) bool {
	switch strings.ToLower(strings.TrimSpace(status)) {
	case "done", "failed", "skipped":
		return true
	default:
		return false
	}
}

func goalStepOK(status string) bool {
	switch strings.ToLower(strings.TrimSpace(status)) {
	case "failed", "skipped":
		return false
	default:
		return true
	}
}

func truncateGoalSummary(text string, max int) string {
	t := strings.TrimSpace(text)
	if t == "" {
		return "(step finished with no written summary)"
	}
	if max < 1 {
		max = goalDisplaySummaryCap
	}
	// Prefer rune-aware truncate via existing truncate helper (cells ≈ runes for ASCII).
	if len(t) <= max {
		return t
	}
	return truncate(t, max)
}

func goalStepFingerprint(status, summary string) string {
	s := strings.TrimSpace(summary)
	return strings.ToLower(strings.TrimSpace(status)) + "|" + strconv.Itoa(len(s)) + "|" + truncate(s, 48)
}

func goalPromptLabel(p goalPromptSnap) string {
	title := strings.TrimSpace(p.Title)
	if title == "" {
		title = strings.TrimSpace(p.StepID)
	}
	if title == "" {
		title = "step"
	}
	return title
}

func (s *session) goalProgressCounts() (settled, total int) {
	if s.goalState == nil {
		return 0, 0
	}
	total = len(s.goalState.Prompts)
	for _, p := range s.goalState.Prompts {
		if goalTerminalStatus(p.Status) {
			settled++
		}
	}
	return settled, total
}

// persistGoalStepComplete writes a lasting transcript card for a finished
// goal step. Dedupes via s.goalStepLogged so goal_state diffs and
// goal_step_complete do not double-fire.
func (s *session) persistGoalStepComplete(stepID, title, agent, status, summary string) {
	if s.goalStepLogged == nil {
		s.goalStepLogged = map[string]string{}
	}
	key := stepID
	if key == "" {
		key = title + "|" + agent
	}
	fp := goalStepFingerprint(status, summary)
	if prev, ok := s.goalStepLogged[key]; ok && prev == fp {
		return
	}
	s.goalStepLogged[key] = fp

	label := strings.TrimSpace(title)
	if label == "" {
		label = key
	}
	statusNorm := strings.ToLower(strings.TrimSpace(status))
	if statusNorm == "" {
		if goalStepOK(status) {
			statusNorm = "done"
		} else {
			statusNorm = "failed"
		}
	}
	body := truncateGoalSummary(summary, goalDisplaySummaryCap)
	if body == "" || body == "[no result]" {
		body = "(step finished with no written summary)"
	}
	line := fmt.Sprintf("goal step %s · %s", statusNorm, label)
	if agent != "" {
		line += " (" + agent + ")"
	}
	line += "\n" + body

	kind := blkSuccess
	switch statusNorm {
	case "failed":
		kind = blkWarn
	case "skipped":
		kind = blkInfo
	}
	s.logPersist(kind, line)
}

// persistGoalLifecycle writes a lasting bridge line for goal phase / deploy
// transitions (toasts alone leave the transcript dark).
func (s *session) persistGoalLifecycle(text string) {
	t := strings.TrimSpace(text)
	if t == "" {
		return
	}
	if t == s.goalLastLife {
		return
	}
	s.goalLastLife = t
	s.logPersist(blkInfo, t)
}

// announceGoalComplete writes the single lasting "goal complete" line + toast.
// Both goal_state(done) and goal_phase→done call this; the flag dedupes.
func (s *session) announceGoalComplete() {
	if s.goalCompleteLogged {
		return
	}
	s.goalCompleteLogged = true
	s.persistGoalLifecycle("goal complete")
	s.logSuccess("goal complete")
}

func goalStatusBadge(status string) string {
	switch strings.ToLower(strings.TrimSpace(status)) {
	case "done":
		return "✓"
	case "failed":
		return "✗"
	case "skipped":
		return "–"
	case "running", "in_progress", "active":
		return "◷"
	default:
		return "·"
	}
}

// renderGoalProgressPanel lists goal steps with status during deploy / wrap-up.
func (s *session) renderGoalProgressPanel(w int) string {
	if s.goalState == nil || !goalShowsProgressPanel(s.goalState.Phase, s.goalState.AutoDeploy) {
		return ""
	}
	if w < 20 || s.height < 10 {
		return ""
	}
	settled, total := s.goalProgressCounts()
	phaseLabel := goalProgressPhaseLabel(s.goalState.Phase, s.goalState.AutoDeploy)
	header := fmt.Sprintf("goal · %s · %d/%d", phaseLabel, settled, total)
	var rows []string
	rows = append(rows, accentStyle.Render("◈ ")+boldBaseStyle.Render(header))
	maxRows := min(6, max(2, s.height/4))
	prompts := s.goalState.Prompts
	hidden := 0
	if len(prompts) > maxRows {
		hidden = len(prompts) - maxRows
		prompts = prompts[:maxRows]
	}
	for _, p := range prompts {
		badge := goalStatusBadge(p.Status)
		label := goalPromptLabel(p)
		line := badge + " " + label
		if p.Agent != "" {
			line += " · " + p.Agent
		}
		st := strings.ToLower(strings.TrimSpace(p.Status))
		switch st {
		case "failed":
			rows = append(rows, warnStyle.Render(truncate(line, max(8, w-6))))
		case "done":
			rows = append(rows, successStyle.Render(truncate(line, max(8, w-6))))
		case "running", "in_progress", "active":
			rows = append(rows, accentStyle.Render(truncate(line, max(8, w-6))))
		default:
			rows = append(rows, dimStyle.Render(truncate(line, max(8, w-6))))
		}
		if goalTerminalStatus(p.Status) && strings.TrimSpace(p.Summary) != "" {
			preview := truncate(strings.TrimSpace(p.Summary), max(12, w-10))
			rows = append(rows, dimStyle.Render("  "+preview))
		}
	}
	if hidden > 0 {
		rows = append(rows, dimStyle.Render(fmt.Sprintf("… +%d more steps", hidden)))
	}
	body := strings.Join(rows, "\n")
	boxW := max(1, w-4)
	return lipgloss.NewStyle().
		BorderStyle(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(c.decor)).
		Padding(0, 1).
		Width(boxW).MaxWidth(max(1, w)).
		Render(body)
}

func (s *session) goalProgressPanelHeight() int {
	p := s.renderGoalProgressPanel(s.width)
	if p == "" {
		return 0
	}
	return lipgloss.Height(p)
}
