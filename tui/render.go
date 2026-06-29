package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/charmbracelet/lipgloss"
)

// ---------------------------------------------------------------------------
// Layout & rendering
//
// Top-to-bottom:
//   header   (3)   brand + tagline row, cwd + tip row, separator
//   viewport (N)   the scrollable transcript
//   posbar   (1)   scroll-position bar / "↓ N new" affordance
//   banner   (1)   approval prompt (when pending)
//   panel    (?)   active-tasks box (when a scout is in flight)
//   inputbox (3)   bordered chat input
//   footer   (1)   status line: state · model · metrics · context
//
// The posbar is always reserved so scrolling up never reflows the transcript.
// ---------------------------------------------------------------------------

const (
	headerLines      = 3 // brand row + cwd row + separator
	positionBarLines = 1 // scroll-position / new-messages bar
	footerLines      = 4 // bordered input (3) + status line (1)
	minBodyLines     = 3
)

func (s *session) layout() {
	if !s.ready {
		return
	}
	extra := s.activeTasksHeight()
	if s.pendingApproval != nil {
		extra++
	}
	h := s.height - headerLines - positionBarLines - footerLines - extra
	if h < minBodyLines {
		h = minBodyLines
	}
	// width/height changed → cached wrapped renders are stale; re-render.
	if s.viewport.Width != s.width || s.viewport.Height != h {
		s.invalidateAll()
	}
	s.viewport.Width = s.width
	s.viewport.Height = h
	s.input.Width = s.width - 4
	s.refresh()
}

// renderHeader: a 2-line branded banner — brand + tagline on row 1, the
// working directory + a tip on row 2 (right-aligned). The model/auth state
// moved to the footer so the header stays clean like the reference design.
func (s *session) renderHeader() string {
	brand := accentStyle.Render("◆ ") + boldBaseStyle.Render("Umans") + dimStyle.Render(" harness")
	tagline := mutedStyle.Render("an OpenAI-compatible coding agent")
	row1 := fitRow(s.width, brand, tagline)

	cwd := dimStyle.Render(s.cwd)
	tip := dimStyle.Render("Tip: / for commands · ? for help")
	row2 := fitRow(s.width, cwd, tip)
	return row1 + "\n" + row2
}

// renderPositionBar: a thin scroll affordance. Pinned to the bottom it is a
// subtle dim rule; scrolled up it becomes an accent bar telling the user how
// many newer lines are hidden below and how to jump back.
func (s *session) renderPositionBar() string {
	w := s.width
	if w < 1 {
		w = 1
	}
	// lines hidden below the current viewport window (0 when pinned to bottom)
	below := s.viewport.TotalLineCount() - s.viewport.YOffset - s.viewport.VisibleLineCount()
	if below < 0 {
		below = 0
	}
	if below > 0 {
		pct := int(s.viewport.ScrollPercent() * 100)
		msg := fmt.Sprintf("↓ %d new · %d%% · PgDn scroll · Ctrl+End jump", below, pct)
		return lipgloss.NewStyle().
			Width(w).
			Background(lipgloss.Color(c.accent)).
			Foreground(lipgloss.Color(c.bg)).
			Bold(true).
			Padding(0, 1).
			Render(msg)
	}
	// pinned to bottom: a subtle dim rule with the scroll position centred.
	pct := 100
	bar := fmt.Sprintf(" %d%% ", pct)
	// build: dashes + bar + dashes, centred.
	bl := lipgloss.Width(bar)
	if bl >= w {
		return dimStyle.Render(strings.Repeat("─", w))
	}
	left := (w - bl) / 2
	right := w - bl - left
	if left < 0 {
		left = 0
	}
	if right < 0 {
		right = 0
	}
	return dimStyle.Render(strings.Repeat("─", left)) + mutedStyle.Render(bar) + dimStyle.Render(strings.Repeat("─", right))
}

// approvalBanner: a full-width sticky bar shown while a decision is pending.
func (s *session) renderApprovalBanner() string {
	a := s.pendingApproval
	args := truncate(a.args, max(1, s.width-40))
	msg := fmt.Sprintf("⚠  approve %s(%s)   [y]es   [n]o   [a]lways", a.tool, args)
	return lipgloss.NewStyle().
		Width(s.width).
		Background(lipgloss.Color(c.warn)).
		Foreground(lipgloss.Color(c.bg)).
		Bold(true).
		Padding(0, 1).
		Render(msg)
}

// renderFooter: the status rail under the input box — state · leader · model ·
// approval on the left, throughput metrics centered, context budget on the right.
func (s *session) renderFooter() string {
	var left strings.Builder
	if s.busy {
		left.WriteString(s.spinner.View())
		left.WriteString(" " + accentStyle.Render("working"))
	} else if s.authed {
		left.WriteString(successStyle.Render("● ready"))
	} else if len(s.models) > 0 {
		left.WriteString(warnStyle.Render("● no key"))
	} else {
		left.WriteString(dimStyle.Render("● initializing…"))
	}
	if len(s.models) > 0 && s.modelIdx >= 0 && s.modelIdx < len(s.models) {
		left.WriteString(dimStyle.Render(" · leader · "))
		left.WriteString(boldBaseStyle.Render(s.models[s.modelIdx].ID))
	} else if len(s.models) == 0 {
		left.WriteString(dimStyle.Render(" · leader · ") + dimStyle.Render("no model"))
	}
	left.WriteString(dimStyle.Render(" · " + s.approvalMode()))
	eff := s.settings.ReasoningEffort
	if eff == "" {
		eff = s.preferredLevel(s.thinkingLevels())
	}
	left.WriteString(dimStyle.Render(" · think:" + eff))

	mid := s.renderMetrics()
	midStyled := ""
	if mid != "" {
		midStyled = mutedStyle.Render(mid)
	}
	right := s.renderContext()
	return fitRow3(s.width, left.String(), midStyled, right)
}

// renderMetrics builds the centred throughput string from the last metrics
// event. Only TPS is shown in the chrome (compact) so it fits beside the
// model + context budget; ttft and cumulative tokens live in /stats + the
// debug log. The context bar (renderContext) already shows token usage.
func (s *session) renderMetrics() string {
	if len(s.lastMetrics) == 0 {
		return ""
	}
	var m map[string]json.RawMessage
	if json.Unmarshal(s.lastMetrics, &m) != nil {
		return ""
	}
	tps := get(m, "tps")
	if tps == "" || tps == "null" {
		return ""
	}
	return fmt.Sprintf("%s tok/s", tps)
}

func fitRow3(width int, left, mid, right string) string {
	tl := lipgloss.Width(left)
	tr := lipgloss.Width(right)
	tm := lipgloss.Width(mid)

	// place mid centered, right flush; fall back to left/right if too narrow.
	if tm > 0 && width-(tl+tr+4) >= tm {
		leftMid := (width - tm) / 2
		leftPad := leftMid - tl
		if leftPad < 2 {
			leftPad = 2
		}
		rightPad := width - tl - leftPad - tm - tr
		if rightPad < 2 {
			rightPad = 2
		}
		return left + strings.Repeat(" ", leftPad) + mid + strings.Repeat(" ", rightPad) + right
	}
	return fitRow(width, left, right)
}

// fitRow places left flush and right flush, padding the gap.
func fitRow(width int, left, right string) string {
	tl := lipgloss.Width(left)
	if tl > width {
		return left
	}
	tr := lipgloss.Width(right)
	gap := width - tl - tr
	if gap < 0 {
		return left
	}
	return left + strings.Repeat(" ", gap) + right
}

func (s *session) renderSeparator() string {
	w := s.width
	if w < 1 {
		w = 1
	}
	return separatorStyle.Render(strings.Repeat("─", w))
}

// compactTokens formats a token count compactly: 940 → "940", 1200 → "1.2k".
func compactTokens(n uint64) string {
	if n < 1000 {
		return fmt.Sprintf("%d", n)
	}
	if n < 1_000_000 {
		return fmt.Sprintf("%.1fk", float64(n)/1000)
	}
	return fmt.Sprintf("%.1fM", float64(n)/1_000_000)
}

// cwdBasename returns the last path component of the working dir, for the header.
func cwdBasename() string {
	wd, err := os.Getwd()
	if err != nil {
		return ""
	}
	b := filepath.Base(wd)
	if b == "." || b == string(filepath.Separator) {
		return ""
	}
	return b
}

// cwdDisplay returns the working dir as a short home-relative path (~/rest),
// falling back to the basename when it's long or off-home. Shown in the header.
func cwdDisplay() string {
	wd, err := os.Getwd()
	if err != nil {
		return ""
	}
	if abs, err := filepath.Abs(wd); err == nil {
		wd = abs
	}
	if home, err := os.UserHomeDir(); err == nil && home != "" {
		if wd == home {
			return "~"
		}
		if rel, err := filepath.Rel(home, wd); err == nil && !strings.HasPrefix(rel, "..") {
			return "~/" + filepath.ToSlash(rel)
		}
	}
	return cwdBasename()
}

// renderContext builds the right-aligned context-budget string: "7% 13.7k/128k"
// using the current model's context window and the cumulative session tokens.
func (s *session) renderContext() string {
	var maxToks uint64
	if len(s.models) > 0 && s.modelIdx >= 0 && s.modelIdx < len(s.models) {
		maxToks = uint64(s.models[s.modelIdx].ContextWindow)
	}
	cur := s.contextTokens
	if maxToks == 0 {
		return compactTokens(cur) + " tok"
	}
	pct := int(float64(cur) / float64(maxToks) * 100)
	if pct < 1 && cur > 0 {
		pct = 1
	}
	return fmt.Sprintf("%d%% %s/%s", pct, compactTokens(cur), compactTokens(maxToks))
}

// renderInputBox wraps the chat input in a rounded border so it reads as a
// distinct chat composer (matching the reference design). The prompt glyph is
// dropped; the box itself is the affordance.
func (s *session) renderInputBox() string {
	w := s.width
	if w < 6 {
		w = 6
	}
	inner := s.input.View()
	innerW := w - 4 // "│ " + content + " │"
	if lipgloss.Width(inner) != innerW {
		inner = lipgloss.NewStyle().Width(innerW).MaxWidth(innerW).Render(inner)
	}
	top := "╭" + strings.Repeat("─", w-2) + "╮"
	row := "│ " + inner + " │"
	bot := "╰" + strings.Repeat("─", w-2) + "╯"
	return top + "\n" + row + "\n" + bot
}

// renderActiveTasks draws a bordered "active tasks" panel listing in-flight
// scout (spawn) tool calls with their model and live elapsed time. Returns ""
// when nothing is in flight. Tools run sequentially in the core, so this shows
// real work, not fabricated parallelism.
// ponytail: per-scout token counts aren't emitted by the core; the aggregate
// context budget is shown in the footer instead. Add per-scout metrics if the
// core grows a subagent-usage event.
func (s *session) renderActiveTasks(w int) string {
	var scouts []*block
	for _, b := range s.blocks {
		if isInFlight(b) && b.name == "spawn" {
			scouts = append(scouts, b)
		}
	}
	if len(scouts) == 0 {
		return ""
	}
	if len(scouts) > 3 {
		scouts = scouts[:3]
	}
	var rows []string
	for _, b := range scouts {
		label := taskLabel(b, w-6)
		rows = append(rows, accentStyle.Render("◷ ")+baseStyle.Render(label))
		meta := dimStyle.Render("  " + taskRole(b))
		if m := taskModel(b); m != "" {
			meta += dimStyle.Render(" · ") + mutedStyle.Render(m)
		}
		meta += dimStyle.Render(" · " + formatDur(time.Since(b.started)))
		rows = append(rows, meta)
	}
	body := accentStyle.Render("active tasks") + "\n" + strings.Join(rows, "\n")
	boxW := w - 2
	if boxW < 20 {
		boxW = 20
	}
	return lipgloss.NewStyle().
		BorderStyle(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(c.dim)).
		Padding(0, 1).
		Width(boxW).
		Render(body)
}

// activeTasksHeight is the lines the active-tasks panel claims (0 when none),
// so layout() can shrink the viewport to make room.
func (s *session) activeTasksHeight() int {
	p := s.renderActiveTasks(s.width)
	if p == "" {
		return 0
	}
	return lipgloss.Height(p)
}

func (s *session) View() string {
	if !s.ready {
		return baseStyle.Render("starting core…")
	}
	parts := []string{
		s.renderHeader(),
		s.renderSeparator(),
		s.viewport.View(),
		s.renderPositionBar(),
	}
	if s.pendingApproval != nil {
		parts = append(parts, s.renderApprovalBanner())
	}
	if p := s.renderActiveTasks(s.width); p != "" {
		parts = append(parts, p)
	}
	parts = append(parts, s.renderInputBox(), s.renderFooter())
	view := strings.Join(parts, "\n")
	if s.modal.kind != modalNone {
		return s.renderModalOverlay(view)
	}
	return view
}
