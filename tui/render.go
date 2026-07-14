package main

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
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
	positionBarLines = 1 // scroll-position / new-messages bar
)

func envEnabled(names ...string) bool {
	for _, name := range names {
		switch strings.ToLower(strings.TrimSpace(os.Getenv(name))) {
		case "1", "true", "yes", "on":
			return true
		}
	}
	return false
}

// These environment switches make the TUI usable in assistive/log-oriented
// environments without adding persisted settings that older cores reject.
func prefersReducedMotion() bool {
	return envEnabled("CATCODE_REDUCED_MOTION", "REDUCED_MOTION")
}

func plainTerminalMode() bool {
	return envEnabled("CATCODE_PLAIN", "CATCODE_NO_ALT_SCREEN")
}

var noColorANSIRe = regexp.MustCompile(`\x1b\[[0-9;?]*[A-Za-z]`)

// headerHeight is deliberately measured rather than hard-coded: compact
// terminals use a single header row, while normal terminals retain both rows.
func (s *session) headerHeight() int { return lipgloss.Height(s.renderHeader()) + 1 }

// relayoutHeights recomputes the viewport height to fit the current input-box
// height + panels and applies it. It is CHEAP: it does not re-render the
// transcript blocks (their content is unchanged; only the viewport's visible
// window height moves). Called after every input update so a growing
// multi-line input shrinks the viewport instead of pushing the footer off the
// bottom of the screen.
func (s *session) relayoutHeights() {
	if !s.ready {
		return
	}
	// Fixed-height optional panels (everything except the active-tasks panel,
	// whose entry count we cap below to fit).
	fixedExtra := 0
	if s.coreLifecycle == coreFailed && s.hasConversation() {
		fixedExtra++
	}
	if s.updateInfo != nil {
		fixedExtra++
	}
	if s.pendingApproval != nil {
		fixedExtra += s.approvalHeight()
	}
	if s.pendingIntercom != nil {
		fixedExtra++
	}
	fixedExtra += s.mentionFlyoutHeight()
	fixedExtra += s.todoPanelHeight() + s.queueBannerHeight()
	fixedExtra += s.toastHeight() + s.oauthBannerHeight()
	// Space left for the viewport + the active-tasks panel, leaving 1 line of
	// slack for v2's cursed renderer (it scrolls/overlaps when the view fills
	// the terminal exactly).
	avail := s.height - s.headerHeight() - positionBarLines - s.footerHeight() - fixedExtra - 1
	// Cap the active-tasks panel so it (plus a 1-line viewport) fits the
	// available height. Each entry renders up to 2 rows; the panel adds 3
	// (border + header). Measured AFTER setting the cap so activeTasksHeight()
	// reflects the truncated panel.
	minViewport := 3
	if s.height < 14 {
		minViewport = 1
	}
	const perEntry, panelOverhead = 2, 3
	fit := (avail - panelOverhead - minViewport) / perEntry
	if fit > 4 {
		fit = 4
	}
	if fit < 0 {
		fit = 0
	}
	if s.pendingApproval != nil {
		fit = 0 // approvals take priority; active tasks remain in the transcript
	}
	s.maxTaskRows = fit
	tasksH := s.activeTasksHeight()
	h := avail - tasksH
	if h < 0 {
		h = 0 // panels fill the screen; hide the transcript rather than overflow
	}
	s.viewport.SetWidth(s.width)
	s.viewport.SetHeight(h)
	s.input.SetWidth(max(1, s.width-4))
}

// layout recomputes heights AND re-renders the transcript. Use it on events
// that change or re-wrap the blocks (terminal resize, task start/finish). For
// input-only changes (typing/pasting) use relayoutHeights() — re-rendering
// every keystroke is expensive.
func (s *session) layout() {
	prevW, prevH := s.viewport.Width(), s.viewport.Height()
	s.relayoutHeights()
	// width/height changed → cached wrapped renders are stale; re-render.
	if s.viewport.Width() != prevW || s.viewport.Height() != prevH {
		s.invalidateAll()
	}
	s.refresh()
}

func (s *session) renderIntercomBanner() string {
	i := s.pendingIntercom
	sendKey, closeKey := s.keyHint("send"), s.keyHint("close")
	if sendKey == "" {
		sendKey = "send"
	}
	if closeKey == "" {
		closeKey = "skip"
	}
	var msg string
	if !s.intercomNudge.IsZero() && time.Since(s.intercomNudge) < 1500*time.Millisecond {
		msg = fmt.Sprintf("⚠ type a reply below, then %s   ·   %s to skip", sendKey, closeKey)
	} else {
		queue := ""
		if n := 1 + len(s.intercomQueue); n > 1 {
			queue = fmt.Sprintf(" [1 of %d]", n)
		}
		msg = fmt.Sprintf("❓ subagent %s%s asks: %s   type reply + %s   ·   %s skip", i.from, queue, truncate(i.message, max(1, s.width-60)), sendKey, closeKey)
	}
	return lipgloss.NewStyle().
		Width(max(1, s.width-2)).MaxWidth(max(1, s.width)).
		Background(lipgloss.Color(c.accent)).
		Foreground(lipgloss.Color(c.bg)).
		Bold(true).
		Padding(0, 1).
		Render(msg)
}

func (s *session) renderCoreFailureBanner() string {
	if s.coreLifecycle != coreFailed || !s.hasConversation() {
		return ""
	}
	msg := "core unavailable · r retry · q quit"
	if s.coreFailure != "" && s.width >= 64 {
		msg = truncate(s.coreFailure+" · r retry · q quit", max(1, s.width-2))
	}
	return lipgloss.NewStyle().MaxWidth(max(1, s.width)).
		Foreground(lipgloss.Color(c.bg)).Background(lipgloss.Color(c.err)).Bold(true).
		Render(" " + msg)
}

// renderHeader: a 2-line branded banner — brand + tagline on row 1, the
// working directory + a tip on row 2 (right-aligned). The model/auth state
// moved to the footer so the header stays clean like the reference design.
func (s *session) renderHeader() string {
	brand := accentStyle.Render("◆ ") + boldBaseStyle.Render("Catalyst") + dimStyle.Render(" Code")
	if s.width < 50 {
		// On small screens identity and location are the only durable context.
		// Tips and taglines remain available in help and would otherwise wrap.
		cwd := dimStyle.Render(truncatePath(s.cwd, max(1, s.width-16)))
		return lipgloss.NewStyle().MaxWidth(max(1, s.width)).Render(fitRow(s.width, brand, cwd))
	}
	tagline := mutedStyle.Render("a multi-provider coding agent")
	row1 := fitRow(s.width, brand, tagline)

	cwd := dimStyle.Render(truncatePath(s.cwd, max(20, s.width-40)))
	tip := dimStyle.Render("Tip: / for commands · ! for bash · ? for help")
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
	below := s.viewport.TotalLineCount() - s.viewport.YOffset() - s.viewport.VisibleLineCount()
	if below < 0 {
		below = 0
	}
	if below > 0 {
		pct := int(s.viewport.ScrollPercent() * 100)
		msg := fmt.Sprintf("↓ %d new · %d%% · PgDn scroll · Ctrl+End jump", below, pct)
		return lipgloss.NewStyle().
			Width(max(1, w-2)).MaxWidth(w).
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
	var actions []string
	if key := s.keyHint("approve"); key != "" {
		actions = append(actions, "["+key+"] approve")
	}
	if key := s.keyHint("deny"); key != "" {
		actions = append(actions, "["+key+"] deny")
	}
	if key := s.keyHint("approve_always"); key != "" {
		actions = append(actions, "["+key+"] always")
	}
	controls := strings.Join(actions, " · ")
	avail := s.width - lipgloss.Width(controls) - 24
	if avail < 8 {
		avail = 8
	}
	summary := truncate(approvalSummary(a.tool, a.args), avail)
	msg := "⚠  approve  " + toolIcon(a.tool) + " " + toolDisplayName(a.tool) + "  " + summary
	if controls != "" {
		msg += "   " + controls
	}
	banner := lipgloss.NewStyle().
		Width(max(1, s.width-2)).MaxWidth(max(1, s.width)).
		Background(lipgloss.Color(c.warn)).
		Foreground(lipgloss.Color(c.bg)).
		Bold(true).
		Padding(0, 1).
		Render(msg)
	if strings.TrimSpace(a.diff) != "" {
		banner += "\n" + s.renderApprovalDiff(a)
	}
	return banner
}

func (s *session) renderApprovalDiff(a *approvalPrompt) string {
	if !a.expanded {
		return renderDiffPanel(a.diff, false, s.width, s.keyHint("toggle_tool_output"))
	}
	all := strings.Split(renderDiffPanel(a.diff, true, s.width, s.keyHint("toggle_tool_output")), "\n")
	capRows := s.height / 2
	if capRows < 3 {
		capRows = 3
	}
	if capRows > len(all) {
		capRows = len(all)
	}
	maxScroll := len(all) - capRows
	if a.diffScroll > maxScroll {
		a.diffScroll = maxScroll
	}
	if a.diffScroll < 0 {
		a.diffScroll = 0
	}
	view := strings.Join(all[a.diffScroll:a.diffScroll+capRows], "\n")
	if len(all) > capRows {
		view += "\n" + dimStyle.Render(fmt.Sprintf("│ diff rows %d–%d/%d · PgUp/PgDn scroll", a.diffScroll+1, a.diffScroll+capRows, len(all)))
	}
	return view
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
	if s.width < 50 {
		return s.renderCompactFooter()
	}
	// Line 1 — state & identity.
	var left strings.Builder
	if s.coreLifecycle == coreFailed {
		left.WriteString(errStyle.Render("● core unavailable"))
	} else if s.busy {
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
	left.WriteString(dimStyle.Render(" · " + approvalModeLabel(s.approvalMode())))
	eff := s.settings.ReasoningEffort
	if eff == "" {
		eff = s.preferredLevel(s.thinkingLevels())
	}
	left.WriteString(dimStyle.Render(" · think:" + eff))
	if s.settings.MouseWheel {
		left.WriteString(dimStyle.Render(" · mouse:on"))
	}
	if s.pluginStatus != "" {
		left.WriteString(dimStyle.Render(" · "))
		left.WriteString(mutedStyle.Render(s.pluginStatus))
	}
	statusLine := lipgloss.NewStyle().MaxWidth(max(1, s.width)).Render(left.String())

	// Line 2 — metrics (left) + context budget (right). Both can be empty
	// before the first turn; then we emit just the status line.
	mid := s.renderMetrics()
	right := s.renderContext()
	if mid == "" && right == "" {
		return statusLine
	}
	styledMid := mutedStyle.Render(mid)
	second := fitRow(s.width, styledMid, right)
	// Context pressure is the safety-critical field. On narrow terminals keep it
	// instead of letting fitRow silently discard the right-hand side.
	if right != "" && lipgloss.Width(styledMid)+lipgloss.Width(right)+1 > s.width {
		second = right
	}
	return statusLine + "\n" + second
}

// renderCompactFooter keeps the safety-critical state/context visible in one
// row. Lower-priority approval, reasoning, plugin and mouse details remain in
// the settings/status modals instead of pushing the composer off tiny screens.
func (s *session) renderCompactFooter() string {
	var state string
	switch {
	case s.coreLifecycle == coreFailed:
		state = errStyle.Render("● core down")
	case s.busy:
		if prefersReducedMotion() {
			state = accentStyle.Render("● working")
		} else {
			state = s.spinner.View() + " " + accentStyle.Render("working")
		}
	case s.authed:
		state = successStyle.Render("● ready")
	case len(s.models) > 0:
		state = warnStyle.Render("● no key")
	default:
		state = dimStyle.Render("● starting")
	}

	right := ""
	if len(s.models) > 0 && s.modelIdx >= 0 && s.modelIdx < len(s.models) {
		right = boldBaseStyle.Render(truncate(s.models[s.modelIdx].ID, max(4, s.width-lipgloss.Width(state)-3)))
	}
	if s.contextTokens > 0 {
		var maxToks uint64
		if len(s.models) > 0 && s.modelIdx >= 0 && s.modelIdx < len(s.models) {
			maxToks = uint64(s.models[s.modelIdx].ContextWindow)
		}
		ctx := compactTokens(s.contextTokens)
		if maxToks > 0 {
			pct := min(100, int(float64(s.contextTokens)/float64(maxToks)*100))
			ctx = fmt.Sprintf("%d%% %s/%s", pct, ctx, compactTokens(maxToks))
		}
		if lipgloss.Width(state)+lipgloss.Width(ctx)+1 <= s.width {
			right = mutedStyle.Render(ctx)
		}
	}
	return lipgloss.NewStyle().MaxWidth(max(1, s.width)).Render(fitRow(s.width, state, right))
}

// renderMetrics builds the throughput string for the footer's second line:
// TPS (rounded to the nearest integer) and TTFT, plus the prefix-cache hit rate
// (e.g. "42 tok/s · 180ms ttft · 87% cached"). During an in-flight stream the
// core may emit tps_est, an approximate live throughput based on streamed text;
// final metrics use tps, the provider-reported real token count.
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
		approx := false
		if tps == "" || tps == "null" {
			tps = get(m, "tps_est")
			approx = tps != "" && tps != "null"
		}
		if tps != "" && tps != "null" {
			// Round to the nearest integer so the footer reads "71 tok/s"
			// rather than "71.123132991239 tok/s". Prefix live estimates with
			// "~" so they are useful in-flight without being confused for the
			// final provider-usage-derived TPS.
			prefix := ""
			if approx {
				prefix = "~"
			}
			if f, err := strconv.ParseFloat(tps, 64); err == nil {
				out = fmt.Sprintf("%s%d tok/s", prefix, int(math.Round(f)))
			} else {
				out = fmt.Sprintf("%s%s tok/s", prefix, tps)
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
	// Context-management reclaim: cumulative tokens freed by digest + compaction
	// and the current rolling summary's size, shown next to the cache stat so the
	// cost/benefit of compaction is visible at a glance.
	if s.tokensSaved > 0 {
		if out != "" {
			out += " · "
		}
		out += fmt.Sprintf("%s saved", compactTokens(s.tokensSaved))
	}
	if s.summaryChars > 0 {
		if out != "" {
			out += " · "
		}
		out += fmt.Sprintf("summary %s chars", compactTokens(uint64(s.summaryChars)))
	}
	// Live Umans account-wide concurrency (used/limit) goes FIRST, ahead of
	// tps/ttft/cached, so it reads "Conc 3/8 · 42 tok/s · …". It is shown even
	// when idle (no turn metrics) because it is polled independently every few
	// seconds — that is the "always live" part. Hidden when not Umans / fetch
	// failed; limit renders ∞ when the plan is unlimited.
	if conc := s.renderUmansConc(); conc != "" {
		if out != "" {
			out = conc + " · " + out
		} else {
			out = conc
		}
	}
	return out
}

// renderUmansConc renders the live concurrency field for the footer, e.g.
// "Conc 3/8". Returns "" (hide) when there is no usage reading (not Umans,
// no key, or the /v1/usage fetch failed), OR when the selected model does NOT
// route to the Umans provider the poll is tracking — a Gemini/OpenAI model
// selected means no conc field, even if a Umans provider is logged in. A null
// limit (unlimited plan) renders as ∞.
func (s *session) renderUmansConc() string {
	if s.umansConcUsed == nil || s.umansConcProvider == "" {
		return ""
	}
	// Only show when the selected model routes to this Umans provider.
	if s.modelIdx < 0 || s.modelIdx >= len(s.models) {
		return ""
	}
	if s.models[s.modelIdx].Provider != s.umansConcProvider {
		return ""
	}
	if s.umansConcLimit == nil {
		return fmt.Sprintf("Conc %d/∞", *s.umansConcUsed)
	}
	return fmt.Sprintf("Conc %d/%d", *s.umansConcUsed, *s.umansConcLimit)
}

// fitRow places left flush and right flush, padding the gap.
func fitRow(width int, left, right string) string {
	if width < 1 {
		return ""
	}
	tl := lipgloss.Width(left)
	if tl > width {
		return lipgloss.NewStyle().MaxWidth(width).Render(left)
	}
	tr := lipgloss.Width(right)
	gap := width - tl - tr
	if gap < 0 {
		return lipgloss.NewStyle().MaxWidth(width).Render(left)
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

// composerPlaceholder returns the empty-input hint, contextualized for busy /
// approval / queue so in-flight controls aren't invisible.
func (s *session) composerPlaceholder() string {
	if s.coreLifecycle == coreStarting && s.coreStartGen > 0 {
		return "Starting core and checking credentials…"
	}
	if s.coreLifecycle == coreFailed {
		return "Core unavailable — r retry · q quit · see the recovery panel"
	}
	if s.pendingApproval != nil {
		return "Type a follow-up, or clear input to use the approval keys…"
	}
	if s.busy {
		send, close := s.keyHint("send"), s.keyHint("close")
		if send == "" {
			send = "Send"
		}
		if close == "" {
			close = "Close"
		}
		if s.queued != nil {
			return "Queue full — " + close + " cancels queued · again aborts…"
		}
		steer := s.keyHint("steer")
		if steer == "" {
			steer = "Ctrl+Enter"
		}
		return send + " queues · " + close + " aborts · " + steer + " steers · / commands"
	}
	if !s.authed {
		return "Log in first — /login · / for commands · ? help"
	}
	if s.input.Placeholder != "" {
		return s.input.Placeholder
	}
	return "Chat with the agent…  (/ commands · ? help)"
}

// keyLabel returns the live binding string for an action, or "".
func (s *session) keyLabel(action string) string {
	if s.keybinds == nil {
		return ""
	}
	return s.keybinds[action]
}

// renderInputBox wraps the chat input in a rounded border so it reads as a
// distinct chat composer. Unlike a plain textinput (which scrolls the line
// horizontally when it overflows), the value is soft-wrapped to the box width
// and the box grows downward so the whole message stays visible. The cursor
// is placed on the correct wrapped line via the textinput's own cursor.Model
// (blink / focus-blur behavior stays identical to the stock textinput).
func (s *session) renderInputBox() string {
	w := s.width
	if w < 1 {
		w = 1
	}
	if w < 4 {
		return lipgloss.NewStyle().MaxWidth(w).Render(s.inputContent(w))
	}
	innerW := w - 4 // "│ " + content + " │"
	// Attachment chips sit above the typed text so pasted images are visible
	// even when the text field is empty (image-only send).
	var chipLine string
	if n := len(s.pendingImages); n > 0 {
		parts := make([]string, 0, n)
		for i := 0; i < n; i++ {
			parts = append(parts, s.pendingImageLabel(i))
		}
		chip := strings.Join(parts, " ")
		// Hint for detaching — only when there's room.
		detach := s.keyHint("detach_image")
		hint := ""
		if detach != "" {
			hint = "  " + detach + " remove"
		}
		if lipgloss.Width(chip)+lipgloss.Width(hint) <= innerW {
			chipLine = accentStyle.Render(chip) + dimStyle.Render(hint)
		} else {
			chipLine = accentStyle.Render(truncate(chip, innerW))
		}
	}
	content := s.inputContent(innerW)
	var lines []string
	if chipLine != "" {
		lines = append(lines, chipLine)
	}
	lines = append(lines, strings.Split(content, "\n")...)
	// Busy / approval hint under the typed text when the box has content so
	// controls stay discoverable even after the placeholder is gone.
	if hint := s.composerHintLine(innerW); hint != "" && s.input.Value() != "" {
		lines = append(lines, hint)
	}
	if s.busy && !prefersReducedMotion() {
		return s.renderInputBoxAnimated(w, innerW, lines)
	}
	top := "╭" + strings.Repeat("─", w-2) + "╮"
	bot := "╰" + strings.Repeat("─", w-2) + "╯"
	var b strings.Builder
	b.WriteString(top)
	for _, ln := range lines {
		pad := innerW - lipgloss.Width(ln)
		if pad < 0 {
			pad = 0
		}
		b.WriteString("\n│ " + ln + strings.Repeat(" ", pad) + " │")
	}
	b.WriteString("\n" + bot)
	return b.String()
}

// composerHintLine is a dim second line inside the composer while busy/queued/
// approval is active and the user is already typing (placeholder is hidden).
func (s *session) composerHintLine(innerW int) string {
	var text string
	switch {
	case s.pendingApproval != nil:
		var parts []string
		for _, action := range []struct{ id, label string }{{"approve", "approve"}, {"deny", "deny"}, {"approve_always", "always"}} {
			if key := s.keyHint(action.id); key != "" {
				parts = append(parts, key+" "+action.label)
			}
		}
		parts = append(parts, "clear input first")
		text = strings.Join(parts, " · ")
	case s.queued != nil:
		text = "queue full"
		if key := s.keyHint("close"); key != "" {
			text += " · " + key + " cancels queued"
		}
	case s.busy:
		steer := s.keyHint("steer")
		if steer == "" {
			steer = "Ctrl+Enter"
		}
		text = s.keyHint("send") + " queues · " + s.keyHint("close") + " aborts · " + steer + " steers"
	default:
		return ""
	}
	return dimStyle.Render(truncateRunes(text, innerW))
}

// renderInputBoxAnimated draws the input box with a "comet": a soft accent
// light that sweeps the box perimeter while a turn is in flight — the TUI
// analog of the web's composer-inflight flowing-gradient ring.
//
// Anti-jank design (see the tui-animation-infrastructure memory):
//   - geometry is identical to the idle border (same ╭─╮│╰╯ chars + counts);
//     only each cell's foreground color changes → zero layout shift;
//   - 24-bit truecolor per cell via lipgloss (auto-downscaled on 256-color
//     terminals), blended between the theme's dim (faded) and accent (bright
//     head) so the sweep is a smooth gradient, not an ANSI stair-step;
//   - the head position is derived from wall-clock time, not a frame count,
//     so dropped frames skip the comet ahead at constant speed instead of
//     slowing/stuttering — the single biggest smoothness lever;
//   - re-renders piggyback on the existing spinner tick (10 FPS, already
//     running while busy) — no new timer, no new re-render storm, and idle
//     (s.busy == false) is a pure no-op that falls through to the plain border.
func (s *session) renderInputBoxAnimated(w, innerW int, lines []string) string {
	H := len(lines)
	P := 2 * (w + H) // perimeter length in cells

	// One full lap per inflightCycle; phase ∈ [0,1) is wall-clock driven.
	phase := float64(time.Now().UnixNano()%int64(inflightCycle)) / float64(int64(inflightCycle))
	head := phase * float64(P)

	// Precompute a smooth brightness ramp of lipgloss styles from dim→accent.
	// Quantizing to a modest number of levels is visually indistinguishable for
	// a soft gaussian glow but avoids building one style object per cell.
	base := hexRGB(c.dim)
	accent := hexRGB(c.accent)
	sigma := float64(P) / inflightSigmaDiv
	ramp := make([]lipgloss.Style, inflightLevels)
	for i := 0; i < inflightLevels; i++ {
		t := float64(i) / float64(inflightLevels-1)
		rgb := blendRGB(base, accent, t)
		ramp[i] = lipgloss.NewStyle().Foreground(lipgloss.Color(fmt.Sprintf("#%02x%02x%02x", rgb[0], rgb[1], rgb[2])))
	}

	// styleAt returns the ramp style for a perimeter cell at index idx, given a
	// head that has swept to `head`. Distance wraps around the ring symmetrically.
	styleAt := func(idx int) lipgloss.Style {
		d := float64(idx) - head
		if d < -float64(P)/2 {
			d += float64(P)
		} else if d > float64(P)/2 {
			d -= float64(P)
		}
		if d < 0 {
			d = -d
		}
		t := math.Exp(-(d * d) / (2 * sigma * sigma))
		li := int(t*float64(inflightLevels-1) + 0.5)
		if li < 0 {
			li = 0
		} else if li >= inflightLevels {
			li = inflightLevels - 1
		}
		return ramp[li]
	}
	render := func(idx int, ch string) string { return styleAt(idx).Render(ch) }

	var b strings.Builder
	// Top edge (clockwise): ╭ at 0, ─ at 1..w-2, ╮ at w-1.
	b.WriteString(render(0, "╭"))
	for i := 1; i < w-1; i++ {
		b.WriteString(render(i, "─"))
	}
	b.WriteString(render(w-1, "╮"))
	// Middle rows: left │ (left edge) + content + right │ (right edge).
	for r, ln := range lines {
		pad := innerW - lipgloss.Width(ln)
		if pad < 0 {
			pad = 0
		}
		rightIdx := w + r      // right edge, traversed top→bottom
		leftIdx := P - (r + 1) // left edge, wraps to meet ╭ at index 0
		b.WriteString("\n")
		b.WriteString(render(leftIdx, "│"))
		b.WriteString(" " + ln + strings.Repeat(" ", pad) + " ")
		b.WriteString(render(rightIdx, "│"))
	}
	// Bottom edge. The perimeter runs clockwise (right→left along the bottom
	// for index continuity with the right edge), but the string is written
	// left→right for display — so ╰ is leftmost and ╯ rightmost.
	b.WriteString("\n")
	for j := 0; j < w; j++ {
		idx := w + H + (w - 1 - j)
		var ch string
		switch {
		case j == w-1:
			ch = "╯"
		case j == 0:
			ch = "╰"
		default:
			ch = "─"
		}
		b.WriteString(render(idx, ch))
	}
	return b.String()
}

// inflight animation tuning. The cycle matches the web's 3s gradient sweep;
// the glow half-width scales with the box perimeter so it reads as a single
// moving light rather than the whole border pulsing.
const (
	inflightCycle    = 3 * time.Second // one comet lap
	inflightSigmaDiv = 12.0            // glow = perimeter / sigmaDiv
	inflightLevels   = 32              // brightness ramp steps (smooth on truecolor)
)

// hexRGB parses a #RRGGBB string into its RGB components.
func hexRGB(hex string) [3]int {
	hex = strings.TrimPrefix(hex, "#")
	if len(hex) != 6 {
		return [3]int{}
	}
	n, err := strconv.ParseUint(hex, 16, 32)
	if err != nil {
		return [3]int{}
	}
	return [3]int{int(n >> 16 & 255), int(n >> 8 & 255), int(n & 255)}
}

// blendRGB linearly interpolates between base and target by t∈[0,1].
func blendRGB(base, target [3]int, t float64) [3]int {
	return [3]int{
		int(math.Round(float64(base[0]) + float64(target[0]-base[0])*t)),
		int(math.Round(float64(base[1]) + float64(target[1]-base[1])*t)),
		int(math.Round(float64(base[2]) + float64(target[2]-base[2])*t)),
	}
}

// maxInputLines caps the input box height: a very long message shows a
// cursor-centered window (with … markers) instead of consuming the screen.
const maxInputLines = 5

// inputContent renders the chat input value soft-wrapped to width w, with the
// textinput cursor cell placed on the correct wrapped line. Returns the
// placeholder when the value is empty. textinput v2 no longer exposes its
// internal Cursor (SetChar/View) or top-level TextStyle/PlaceholderStyle
// fields, so composer text and cursor colors are derived directly from the
// active theme instead of inheriting the terminal's default grey.
func (s *session) inputContent(w int) string {
	if w < 1 {
		w = 1
	}
	value := s.input.Value()
	// Active style state depends on focus; v2 keeps Focused()/Styles().
	st := s.input.Styles()
	active := st.Focused
	if !s.input.Focused() {
		active = st.Blurred
	}
	if value == "" {
		ph := s.composerPlaceholder()
		// When a subagent is waiting on an intercom reply, make it obvious the
		// chat box below is where you type it (the banner alone reads as
		// "press ↵ to reply", which leads users to hit Enter on an empty box).
		if s.pendingIntercom != nil {
			ph = "Reply to " + s.pendingIntercom.from + "…"
		}
		if ph == "" {
			return ""
		}
		return active.Placeholder.Render(truncateRunes(ph, w))
	}
	pos := s.input.Position()
	r := []rune(value)
	if pos < 0 {
		pos = 0
	}
	if pos > len(r) {
		pos = len(r)
	}
	before := r[:pos]
	after := r[pos:] // after[0] is the char under the cursor (if any)

	// beforeLines are the display lines strictly above the cursor's line.
	// wrapRunesMultiline splits on literal '\n' first, then width-wraps each
	// segment, so typed/pasted line breaks render as their own rows instead
	// of being treated as width-1 runes (which broke the box + cursor math).
	beforeLines := wrapRunesMultiline(before, w)
	cLine := len(beforeLines) - 1
	cCol := len(beforeLines[cLine])
	// If the last before-line is exactly full, the cursor wraps to a fresh line.
	if cCol >= w {
		beforeLines = append(beforeLines, []rune{})
		cLine++
		cCol = 0
	}
	// The cursor cell + the remainder after it. When the cursor sits directly on
	// a line break, show an empty cell and force everything that follows onto
	// subsequent display lines (the '\n' just ends the current line).
	curChar := " "
	rest := []rune(nil)
	newlineConsumed := false
	if len(after) > 0 {
		if after[0] == '\n' {
			newlineConsumed = true
			rest = after[1:]
		} else {
			curChar = string(after[0])
			rest = after[1:]
		}
	}
	// How much of `rest` fits on the cursor line (after the cursor cell). It
	// also must not cross a '\n' (a line break ends the cursor line).
	avail := w - cCol - 1
	if avail < 0 {
		avail = 0
	}
	limit := avail
	for i, ch := range rest {
		if ch == '\n' {
			if i < limit {
				limit = i
			}
			break
		}
	}
	if limit > len(rest) {
		limit = len(rest)
	}
	restOnLine := rest[:limit]
	var restAfter []rune
	if limit < len(rest) && rest[limit] == '\n' {
		// The line break ends the cursor line; consume it so it doesn't render
		// as a spurious empty row. The content after it starts the next line.
		newlineConsumed = true
		restAfter = rest[limit+1:]
	} else {
		restAfter = rest[limit:]
	}
	// Display lines below the cursor line. A consumed newline guarantees at
	// least one following line (even when empty) so the break stays visible.
	var restLines [][]rune
	if newlineConsumed || len(restAfter) > 0 {
		restLines = wrapRunesMultiline(restAfter, w)
	}

	styleText := composerTextStyle().Render
	out := make([]string, 0, cLine+1+len(restLines)+1)
	for i := 0; i < cLine; i++ {
		out = append(out, styleText(string(beforeLines[i])))
	}
	// cursor line: text before cursor + cursor cell + text after (on this line).
	// v2 dropped textinput's internal Cursor.SetChar/View. Use explicit theme
	// colors here: Reverse(true) inherits the terminal's foreground/background,
	// which produced the same light-grey cursor in every theme.
	line := styleText(string(beforeLines[cLine]))
	if s.input.Focused() {
		line += composerCursorStyle().Render(curChar)
	} else {
		line += styleText(curChar)
	}
	if len(restOnLine) > 0 {
		line += styleText(string(restOnLine))
	}
	out = append(out, line)
	for _, rl := range restLines {
		out = append(out, styleText(string(rl)))
	}
	// Cap the box height: if the wrapped message is very long, show a window
	// centered on the cursor line so the box never eats the whole screen
	// (the cursor always stays visible). “…” markers flag hidden content.
	cursorIdx := cLine
	if len(out) > maxInputLines {
		half := maxInputLines / 2
		start := cursorIdx - half
		if start < 0 {
			start = 0
		}
		end := start + maxInputLines
		if end > len(out) {
			end = len(out)
			start = max(0, end-maxInputLines)
		}
		var capped []string
		if start > 0 {
			capped = append(capped, dimStyle.Render("…"))
		}
		capped = append(capped, out[start:end]...)
		if end < len(out) {
			capped = append(capped, dimStyle.Render("…"))
		}
		out = capped
	}
	return strings.Join(out, "\n")
}

func composerTextStyle() lipgloss.Style {
	if colorsDisabled() {
		return lipgloss.NewStyle().Inline(true)
	}
	return lipgloss.NewStyle().Foreground(lipgloss.Color(c.fg)).Inline(true)
}

func composerCursorStyle() lipgloss.Style {
	if colorsDisabled() {
		return lipgloss.NewStyle().Reverse(true)
	}
	return lipgloss.NewStyle().
		Foreground(lipgloss.Color(c.bg)).
		Background(lipgloss.Color(c.accent))
}

// wrapRunesMultiline splits r on literal '\n' then width-wraps each segment
// via wrapRunes, producing the full set of display rows for a value that may
// contain typed/pasted line breaks. A trailing newline yields a final empty
// row (so the user sees the blank line they entered). Never returns empty.
func wrapRunesMultiline(r []rune, w int) [][]rune {
	if w < 1 {
		w = 1
	}
	var out [][]rune
	start := 0
	for i, ch := range r {
		if ch == '\n' {
			out = append(out, wrapRunes(r[start:i], w)...)
			start = i + 1
		}
	}
	out = append(out, wrapRunes(r[start:], w)...)
	if len(out) == 0 {
		out = [][]rune{{}}
	}
	return out
}

// wrapRunes hard-wraps a rune slice to at most w runes per line. Hard (not
// word) wrapping keeps the cursor column math exact for the input box.
func wrapRunes(r []rune, w int) [][]rune {
	if w < 1 {
		w = 1
	}
	var lines [][]rune
	for len(r) > 0 {
		n := w
		if n > len(r) {
			n = len(r)
		}
		line := make([]rune, n)
		copy(line, r[:n])
		lines = append(lines, line)
		r = r[n:]
	}
	if len(lines) == 0 {
		lines = [][]rune{{}}
	}
	return lines
}

// inputBoxHeight is the rendered input box height so layout() reserves the
// right number of lines as the box grows with wrapping. Measured from the
// actual render so it can never disagree with View().
func (s *session) inputBoxHeight() int {
	return lipgloss.Height(s.renderInputBox())
}

// footerHeight is the input box + the status rail beneath it.
func (s *session) footerHeight() int {
	return s.inputBoxHeight() + lipgloss.Height(s.renderFooter())
}

// renderActiveTasks draws a bordered "active tasks" panel listing in-flight
// scout (spawn) tool calls with their model and live elapsed time. Returns ""
// when nothing is in flight. Tools run sequentially in the core, so this shows
// real work, not fabricated parallelism.
// ponytail: per-scout token counts aren't emitted by the core; the aggregate
// context budget is shown in the footer instead. Add per-scout metrics if the
// core grows a subagent-usage event.
func (s *session) renderActiveTasks(w int) string {
	if s.height < 12 || len(s.subProgress) == 0 || s.maxTaskRows == 0 {
		return ""
	}
	entries := append([]*subProgressEntry(nil), s.subProgress...)
	// Surface potentially stuck work first; it is more actionable than a
	// freshly-started run when the panel must collapse.
	sort.SliceStable(entries, func(i, j int) bool {
		istuck := entries[i].toolRunning && time.Since(entries[i].toolStart) > 30*time.Second
		jstuck := entries[j].toolRunning && time.Since(entries[j].toolStart) > 30*time.Second
		return istuck && !jstuck
	})
	hidden := 0
	if len(entries) > s.maxTaskRows {
		hidden = len(entries) - s.maxTaskRows
		entries = entries[:s.maxTaskRows]
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
	title := fmt.Sprintf("subagents %d/%d active", len(entries), len(s.subProgress))
	if hidden > 0 {
		title += fmt.Sprintf(" · +%d hidden", hidden)
	}
	body := accentStyle.Render(title) + "\n" + strings.Join(rows, "\n")
	boxW := max(1, w-4) // border + horizontal padding consume four cells
	return lipgloss.NewStyle().
		BorderStyle(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(c.dim)).
		Padding(0, 1).
		Width(boxW).MaxWidth(max(1, w)).MaxHeight(max(1, s.height)).
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

// maxPinnedTodos caps the always-visible todo panel so a long list never eats
// the whole screen (overflow collapses to a "… +N more" hint).
const maxPinnedTodos = 5

// renderTodoPanel draws a persistent, always-visible checklist of the latest
// todo_write state so the plan/progress is glanceable without scrolling the
// transcript. Returns "" when there are no todos.
func (s *session) renderTodoPanel() string {
	if s.height < 12 {
		return ""
	}
	todos := s.todos
	if len(todos) == 0 {
		return ""
	}
	done, pend, run := countTodoStatuses(todos)
	head := accentStyle.Render("tasks") +
		dimStyle.Render(fmt.Sprintf("  · %d items (%d✓ %d○ %d•)", len(todos), done, run, pend))
	if s.pendingApproval != nil {
		return head // preserve decision context; expand the checklist after approval
	}
	// inner content width = boxW - 2(border) - 2(padding); each row indents 2 + "[✓] " 4.
	cw := s.width - 2 - 4 - 2 - 4
	if cw < 4 {
		cw = 4
	}
	rows := make([]string, 0, len(todos))
	for _, t := range todos {
		ck, st := todoCheckbox(get(t, "status"))
		rows = append(rows, "  "+st.Render(ck+" ")+baseStyle.Render(truncate(get(t, "subject"), cw)))
	}
	more := 0
	if len(rows) > maxPinnedTodos {
		more = len(rows) - maxPinnedTodos
		rows = rows[:maxPinnedTodos]
	}
	body := head
	for _, r := range rows {
		body += "\n" + r
	}
	if more > 0 {
		body += "\n" + dimStyle.Italic(true).Render(fmt.Sprintf("  … +%d more", more))
	}
	boxW := max(1, s.width-4) // border + horizontal padding consume four cells
	return lipgloss.NewStyle().
		BorderStyle(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(c.dim)).
		Padding(0, 1).
		Width(boxW).MaxWidth(max(1, s.width)).MaxHeight(max(1, s.height)).
		Render(body)
}

func (s *session) todoPanelHeight() int {
	p := s.renderTodoPanel()
	if p == "" {
		return 0
	}
	return lipgloss.Height(p)
}

// renderQueueBanner is a one-line sticky banner shown while a follow-up or
// steer prompt is buffered (one-deep) behind the running turn. It labels the
// kind and reminds the user Esc cancels just the queued message (vs a bare
// abort). Returns "" when nothing is queued.
func (s *session) renderQueueBanner() string {
	q := s.queued
	if q == nil {
		return ""
	}
	label := "⏳ queued follow-up"
	if q.kind == "steer" {
		label = "⤴ queued steer"
	}
	avail := s.width - len(label) - 24
	if avail < 6 {
		avail = 6
	}
	msg := fmt.Sprintf("%s: %s", label, truncate(q.text, avail))
	if key := s.keyHint("close"); key != "" {
		msg += "   " + key + " to cancel"
	}
	return lipgloss.NewStyle().
		Width(max(1, s.width-2)).MaxWidth(max(1, s.width)).
		Background(lipgloss.Color(c.user)).
		Foreground(lipgloss.Color(c.bg)).
		Bold(true).
		Padding(0, 1).
		Render(msg)
}

func (s *session) queueBannerHeight() int {
	if s.queued == nil {
		return 0
	}
	return 1
}

func (s *session) renderToast() string {
	if s.height < 10 || s.toast == nil {
		return ""
	}
	if time.Now().After(s.toast.until) {
		s.toast = nil
		return ""
	}
	style := mutedStyle
	prefix := "· "
	switch s.toast.kind {
	case toastSuccess:
		style = successStyle
		prefix = "✓ "
	case toastWarn:
		style = warnStyle
		prefix = "! "
	case toastError:
		style = errStyle
		prefix = "✗ "
	}
	msg := truncate(prefix+s.toast.text, max(8, s.width-2))
	return style.Render(msg)
}

func (s *session) toastHeight() int {
	if s.renderToast() == "" {
		return 0
	}
	return 1
}

func (s *session) renderOauthBanner() string {
	o := s.oauth
	if o == nil {
		return ""
	}
	msg := o.message
	if msg == "" {
		msg = "OAuth login"
	}
	extra := ""
	if o.code != "" {
		extra = " · code " + o.code
	}
	line := fmt.Sprintf("🔑 %s%s · URL on clipboard", msg, extra)
	if o.url != "" && s.width > 60 {
		// Show a short URL tail when there's room; full URL is clipboarded.
		u := o.url
		if len(u) > 48 {
			u = u[:45] + "…"
		}
		line = fmt.Sprintf("🔑 %s%s · %s", msg, extra, u)
	}
	return lipgloss.NewStyle().
		Width(max(1, s.width-2)).MaxWidth(max(1, s.width)).
		Background(lipgloss.Color(c.accent)).
		Foreground(lipgloss.Color(c.bg)).
		Bold(true).
		Padding(0, 1).
		Render(truncate(line, max(8, s.width-2)))
}

func (s *session) oauthBannerHeight() int {
	if s.oauth == nil {
		return 0
	}
	return 1
}

func (s *session) View() tea.View {
	var content string
	if !s.ready {
		content = baseStyle.Render("starting core…")
	} else {
		// Recompute the viewport height from the CURRENT input-box + tasks-panel
		// height on every render. This is the single source of truth: paths that
		// mutate the input (insertNewline on Shift+Enter, paste) or the tasks panel
		// (mid-run tool updates) don't all call relayoutHeights themselves, so doing
		// it here guarantees the viewport shrinks to fit before we render — no
		// overflow that pushes the footer off-screen between events. It's cheap
		// (height math only; no transcript block re-render).
		s.relayoutHeights()
		parts := []string{
			s.renderHeader(),
			s.renderSeparator(),
		}
		if b := s.renderCoreFailureBanner(); b != "" {
			parts = append(parts, b)
		}
		if b := s.renderUpdateBanner(); b != "" && s.height >= 10 {
			parts = append(parts, b)
		}
		if o := s.renderOauthBanner(); o != "" {
			parts = append(parts, o)
		}
		parts = append(parts, s.viewport.View(), s.renderPositionBar())
		if p := s.renderTodoPanel(); p != "" {
			parts = append(parts, p)
		}
		if q := s.renderQueueBanner(); q != "" {
			parts = append(parts, q)
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
		if t := s.renderToast(); t != "" {
			parts = append(parts, t)
		}
		parts = append(parts, s.renderInputBox(), s.renderFooter())
		view := strings.Join(parts, "\n")
		if s.modal.kind != modalNone {
			view = s.renderModalOverlay(view)
		}
		// ask flyout: a blocking `ask` prompt renders as a centered overlay on
		// top of the full view (like the modal above). renderAskOverlay is a
		// no-op (returns base unchanged) when s.pendingAsk is nil.
		content = s.renderAskOverlay(view)
		// sudo flyout: a blocking sudo_request (bash command invokes sudo) renders
		// as a centered overlay with a password field. No-op when nil.
		content = s.renderSudoOverlay(content)
		// Final containment is an invariant, not a best effort. Do not paint a
		// background around the multiline view: terminals carry that SGR color
		// across the unused remainder of each row, creating large rectangles after
		// short text. Individual banners/modals still own their intentional fills.
		content = constrainViewContent(content, s.width, s.height)
		if colorsDisabled() {
			content = noColorANSIRe.ReplaceAllString(content, "")
		}
	}
	v := tea.NewView(content)
	// v2 is declarative: alt-screen + mouse mode are View fields, not program
	// options. The renderer also always enables Kitty progressive-keyboard
	// disambiguation + xterm modifyOtherKeys level 2 (restoring them on exit),
	// so modified keys (Shift/Ctrl+Enter, Esc) arrive as real KeyPressMsgs.
	v.AltScreen = !plainTerminalMode()
	if s.settings.MouseWheel {
		v.MouseMode = tea.MouseModeCellMotion
	}
	return v
}

func constrainViewContent(content string, width, height int) string {
	return lipgloss.NewStyle().
		MaxWidth(max(1, width)).
		MaxHeight(max(1, height)).
		Render(content)
}
