package main

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"strconv"
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
//   footer   (2)   line 1: state·model·approval·think  |  line 2: metrics·context
//
// The posbar is always reserved so scrolling up never reflows the transcript.
// ---------------------------------------------------------------------------

const (
	headerLines      = 3 // brand row + cwd row + separator
	positionBarLines = 1 // scroll-position / new-messages bar
	footerLines      = 5 // bordered input (3) + two status lines (identity, metrics/context)
	minBodyLines     = 3
)

func (s *session) layout() {
	if !s.ready {
		return
	}
	extra := s.activeTasksHeight()
	if s.pendingApproval != nil {
		extra += s.approvalHeight()
	}
	if s.pendingIntercom != nil {
		extra++
	}
	extra += s.mentionFlyoutHeight()
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

func (s *session) renderIntercomBanner() string {
	i := s.pendingIntercom
	msg := fmt.Sprintf("❓ subagent %s asks: %s   ↵ reply   esc skip", i.from, truncate(i.message, max(1, s.width-60)))
	return lipgloss.NewStyle().
		Width(s.width).
		Background(lipgloss.Color(c.accent)).
		Foreground(lipgloss.Color(c.bg)).
		Bold(true).
		Padding(0, 1).
		Render(msg)
}

// renderHeader: a 2-line branded banner — brand + tagline on row 1, the
// working directory + a tip on row 2 (right-aligned). The model/auth state
// moved to the footer so the header stays clean like the reference design.
func (s *session) renderHeader() string {
	brand := accentStyle.Render("◆ ") + boldBaseStyle.Render("Umans") + dimStyle.Render(" harness")
	tagline := mutedStyle.Render("an OpenAI-compatible coding agent")
	row1 := fitRow(s.width, brand, tagline)

	cwd := dimStyle.Render(truncatePath(s.cwd, max(20, s.width-40)))
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
// The head reuses the per-tool primitives (icon + name + parsed keyarg) so the
// human approves the actual target ("src/main.rs · 3 replacements") instead
// of a raw JSON blob. For write/edit/patch a unified-diff preview renders
// below so the decision is on the real change, not the search/replace blobs.
func (s *session) renderApprovalBanner() string {
	a := s.pendingApproval
	avail := s.width - 52 // ⚠ approve + icon name + suffix + padding headroom
	if avail < 8 {
		avail = 8
	}
	summary := truncate(approvalSummary(a.tool, a.args), avail)
	msg := "⚠  approve  " + toolIcon(a.tool) + " " + toolDisplayName(a.tool) + "  " +
		summary + "   [y]es   [n]o   [a]lways"
	banner := lipgloss.NewStyle().
		Width(s.width).
		Background(lipgloss.Color(c.warn)).
		Foreground(lipgloss.Color(c.bg)).
		Bold(true).
		Padding(0, 1).
		Render(msg)
	if strings.TrimSpace(a.diff) != "" {
		banner += "\n" + renderDiffPanel(a.diff, false, s.width)
	}
	return banner
}

// renderFooter: the status rail under the input box, split across two lines so
// each carries one concern instead of cramming everything onto a single row:
//
//	line 1 — identity/state: working|ready badge · leader · model · approval · think effort [· mouse]
//	line 2 — performance: throughput + cache hit (left) · context budget (right)
//
// Splitting the old single line lets the metrics breathe and keeps the state
// read glanceable.
func (s *session) renderFooter() string {
	// Line 1 — state & identity.
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
	if s.settings.MouseWheel {
		left.WriteString(dimStyle.Render(" · mouse:on"))
	}
	statusLine := left.String()

	// Line 2 — metrics (left) + context budget (right). Both can be empty
	// before the first turn; then we emit just the status line.
	mid := s.renderMetrics()
	right := s.renderContext()
	if mid == "" && right == "" {
		return statusLine
	}
	return statusLine + "\n" + fitRow(s.width, mutedStyle.Render(mid), right)
}

// renderMetrics builds the throughput string for the footer's second line:
// TPS (rounded to the nearest integer) and TTFT, plus the prefix-cache hit rate
// (e.g. "42 tok/s · 180ms ttft · 87% cached").
//
// The cache rate has a wrinkle: the live mid-stream metrics event omits
// cached_tokens — it only lands in the turn-end metrics. So while a turn is in
// flight there's no cache number for *this* turn yet. We fall back to the
// previous turn's measured rate (captured in s.lastCachePct by the metrics
// handler) and prefix it with "~" so it reads as "from last turn", not a live
// reading. Once the turn-end metrics arrive (cached_tokens present), the fresh,
// un-tilde'd rate is shown.
func (s *session) renderMetrics() string {
	var m map[string]json.RawMessage
	haveLive := len(s.lastMetrics) > 0 && json.Unmarshal(s.lastMetrics, &m) == nil

	var out string
	if haveLive {
		tps := get(m, "tps")
		if tps != "" && tps != "null" {
			// Round to the nearest integer so the footer reads "71 tok/s"
			// rather than "71.123132991239 tok/s".
			if f, err := strconv.ParseFloat(tps, 64); err == nil {
				out = fmt.Sprintf("%d tok/s", int(math.Round(f)))
			} else {
				out = fmt.Sprintf("%s tok/s", tps)
			}
		}
		// Time-to-first-token for this turn (latency, not throughput).
		if ttft := get(m, "ttft_ms"); ttft != "" && ttft != "null" {
			if out != "" {
				out += fmt.Sprintf(" · %sms ttft", ttft)
			} else {
				out = fmt.Sprintf("%sms ttft", ttft)
			}
		}
	}

	// Prefix-cache hit rate. cached_tokens present in the live metrics ⇒ this
	// is the turn-end number (fresh); absent ⇒ mid-stream, so carry the last
	// turn's rate and mark it "~" so it isn't mistaken for a live reading.
	fresh := false
	if haveLive {
		c := get(m, "cached_tokens")
		fresh = c != "" && c != "null" && c != "0"
	}
	if s.lastCachePct > 0 {
		cacheStr := fmt.Sprintf("%d%% cached", s.lastCachePct)
		if !fresh {
			cacheStr = "~" + cacheStr
		}
		if out != "" {
			out += " · " + cacheStr
		} else {
			out = cacheStr
		}
	}
	return out
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
	if pct > 100 {
		pct = 100
	}
	// A 10-cell fill bar tinted by context pressure: green < 60%, amber < 85%,
	// red ≥ 85% — so a glance at the footer shows how full the window is.
	const cells = 10
	filled := cells * pct / 100
	bar := strings.Repeat("▰", filled) + strings.Repeat("▱", cells-filled)
	barStyle := successStyle
	switch {
	case pct >= 85:
		barStyle = errStyle
	case pct >= 60:
		barStyle = warnStyle
	}
	return barStyle.Render(bar) + " " + mutedStyle.Render(fmt.Sprintf("%d%% %s/%s", pct, compactTokens(cur), compactTokens(maxToks)))
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
	if len(s.subProgress) == 0 {
		return ""
	}
	entries := s.subProgress
	if len(entries) > 4 {
		entries = entries[:4]
	}
	var rows []string
	for _, e := range entries {
		elapsed := formatDur(time.Since(e.started))
		head := accentStyle.Render("◷ ") + boldBaseStyle.Render(e.agent) +
			dimStyle.Render(" · "+elapsed+" · "+strconv.Itoa(e.toolCount)+" tools · "+compactTokens(e.tokensIn)+"+"+compactTokens(e.tokensOut)+" tok")
		rows = append(rows, head)
		if e.curTool != "" {
			td := time.Since(e.toolStart)
			icon := toolIcon(e.curTool)
			marker := "  " + icon + " "
			if e.toolRunning && td > 30*time.Second {
				rows = append(rows, warnStyle.Render(marker+"⚠ "+e.curTool+" STUCK "+formatDur(td)))
			} else if e.toolRunning {
				rows = append(rows, dimStyle.Render(marker+e.curTool+" · "+formatDur(td)))
			} else {
				rows = append(rows, dimStyle.Render(marker+e.curTool+" ✓"))
			}
		}
	}
	body := accentStyle.Render("subagents") + "\n" + strings.Join(rows, "\n")
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

// approvalHeight is the lines the sticky approval banner (plus its diff
// preview, when present) claims, so layout() shrinks the viewport to fit.
func (s *session) approvalHeight() int {
	if s.pendingApproval == nil {
		return 0
	}
	return lipgloss.Height(s.renderApprovalBanner())
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
	if s.pendingIntercom != nil {
		parts = append(parts, s.renderIntercomBanner())
	}

	if p := s.renderActiveTasks(s.width); p != "" {
		parts = append(parts, p)
	}
	if f := s.renderMentionFlyout(); f != "" {
		parts = append(parts, f)
	}
	parts = append(parts, s.renderInputBox(), s.renderFooter())
	view := strings.Join(parts, "\n")
	if s.modal.kind != modalNone {
		return s.renderModalOverlay(view)
	}
	return view
}
