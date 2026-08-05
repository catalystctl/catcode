package main

import (
	"net/url"
	"os"
	"os/exec"
	"sort"
	"strconv"
	"strings"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
	"github.com/rivo/uniseg"
)

// ---------------------------------------------------------------------------
// Chrome mouse layer
//
// Everything outside the transcript viewport is "chrome": the header, banners,
// position bar, activity shelf, goal panel, mention flyout, composer and
// footer. chromeHitAt maps a pointer position to the actionable target under
// it using the exact part order of View(), so hit-testing can never disagree
// with what is painted. Clicks use the modal press/release pattern: the press
// records the target, and only a stationary release on the same target
// activates it — a drag is never a click. Hit testing is O(chrome rows): it
// never scans the transcript.
// ---------------------------------------------------------------------------

type chromeZone int

const (
	chromeNone        chromeZone = iota
	chromeHeaderModel            // header model label → model picker
	chromeCoreFailure            // core-unavailable banner → retry / quit
	chromeUpdate                 // update banner → run the self-updater
	chromeOauth                  // sticky OAuth banner → re-copy URL
	chromePositionBar            // scrolled-up position bar → jump to transcript bottom
	chromeShelf                  // activity shelf: approval/intercom/queue/activity
	chromeGoalPanel              // goal progress panel → collapse/expand
	chromeMention                // @-mention flyout → item rows
	chromeInputBox               // composer card → focus + cursor placement
	chromeFooter                 // footer control rail → send/abort/approval/commands
)

// Shelf sub-actions (chromeHit.action for chromeShelf).
const (
	shelfActionToggle    int = iota // collapsed label → expand; expanded header → collapse
	shelfActionGoalLabel            // goal-only shelf row → toggle goal panel
	shelfActionApprovalOnce
	shelfActionApprovalDeny
	shelfActionApprovalAlways
	shelfActionIntercomSubmit
	shelfActionIntercomSkip
	shelfActionQueueCancel
)

// Core-failure banner sub-actions.
const (
	coreFailureRetry int = iota
	coreFailureQuit
)

// chromeHit is the actionable target under a pointer position. action is -1
// for whole-zone targets, otherwise a zone-specific index (mention item row,
// footer control, shelf/banner sub-action).
type chromeHit struct {
	zone   chromeZone
	action int
}

const (
	chromeInputFocus = iota
	chromeInputDetach
)

// chromePress mirrors the modal press-item registry: a press captures the
// target, motion past a small slop marks it as a drag, and a stationary
// release on the same target activates it. Transcript selection never runs
// while a chrome press is active (the two press registries are exclusive).
type chromePress struct {
	active  bool
	hit     chromeHit
	dragged bool
	x, y    int
}

// chromeDragSlop is the pointer travel (manhattan cells) that turns a chrome
// press into a drag, so a slightly jittery click still activates its target.
const chromeDragSlop = 3

func abs(n int) int {
	if n < 0 {
		return -n
	}
	return n
}

// ---------------------------------------------------------------------------
// Blocking ask/sudo overlay clicks
//
// The ask and sudo flyouts render as centered boxes over the entire view and
// own the whole screen while open: transcript selection, chrome targets and
// the wheel are dead behind them. Clicks use the same press/release pattern
// as chrome and modal targets — a press records the target (and question rows
// move focus immediately), and only a stationary release on the same target
// activates submit/skip/approve/decline/cycle actions.
// ---------------------------------------------------------------------------

// overlayClickAction identifies the actionable region of a blocking flyout.
type overlayClickAction int

const (
	overlayClickNone     overlayClickAction = iota
	overlayAskSubmit                        // footer "[send] submit"
	overlayAskSkip                          // footer "[close] skip"
	overlayAskQuestion                      // question block → focus that question
	overlayAskCycleLeft                     // select ‹ arrow → previous option
	overlayAskCycleRight                    // select › arrow → next option
	overlaySudoApprove                      // footer "[send] approve"
	overlaySudoDecline                      // footer "[close] decline"
	overlaySudoPassword                     // password field row → (re)focus it
)

// overlayHit is the target under a pointer cell in a blocking flyout.
type overlayHit struct {
	action overlayClickAction
	index  int // question index (ask question/cycle targets), else -1
}

// overlayPress mirrors chromePress for the blocking flyouts.
type overlayPress struct {
	active  bool
	hit     overlayHit
	dragged bool
	x, y    int
}

// recordOverlayBoxGeom measures a centered flyout box exactly as lipgloss.Place
// will place it and caches the ANSI-free rows for hit-testing.
func (s *session) recordOverlayBoxGeom(box string) (top, left, w, h int, rows []string) {
	w = lipgloss.Width(box)
	h = lipgloss.Height(box)
	top = max(0, (s.height-h)/2)
	left = max(0, (s.width-w)/2)
	rows = plainTranscriptLines(box)
	return top, left, w, h, rows
}

// askOverlayGeom returns the placed ask box geometry and plain rows, computing
// them from a fresh render when the render path has not run yet (e.g. a click
// arriving before the first View after the flyout opened).
func (s *session) askOverlayGeom() (top, left, w, h int, rows []string) {
	if s.askBoxRows != nil {
		return s.askBoxTop, s.askBoxLeft, s.askBoxW, s.askBoxH, s.askBoxRows
	}
	a := s.pendingAsk
	if a == nil || a.form == nil || len(a.questions) == 0 {
		return 0, 0, 0, 0, nil
	}
	box := s.renderAskBox()
	return s.recordOverlayBoxGeom(box)
}

// sudoOverlayGeom mirrors askOverlayGeom for the sudo flyout.
func (s *session) sudoOverlayGeom() (top, left, w, h int, rows []string) {
	if s.sudoBoxRows != nil {
		return s.sudoBoxTop, s.sudoBoxLeft, s.sudoBoxW, s.sudoBoxH, s.sudoBoxRows
	}
	if s.pendingSudo == nil {
		return 0, 0, 0, 0, nil
	}
	box := s.renderSudoBox()
	return s.recordOverlayBoxGeom(box)
}

// overlayBoxCell maps a screen cell into a flyout box, returning box-relative
// (row, col) and whether the cell is inside the clickable interior (past the
// border). Border rows/columns are inert.
func (s *session) overlayBoxCell(x, y, top, left, w, h int) (row, col int, ok bool) {
	if w <= 0 || h <= 0 || y < top || y >= top+h || x < left || x >= left+w {
		return 0, 0, false
	}
	row = y - top
	col = x - left
	if row <= 0 || row >= h-1 || col <= 0 || col >= w-1 {
		return row, col, false
	}
	return row, col, true
}

// overlayHitAt dispatches a screen cell to the open flyout's target table.
// The sudo flyout outranks the ask flyout (both cannot normally be open).
func (s *session) overlayHitAt(x, y int) (overlayHit, bool) {
	if s.pendingSudo != nil {
		return s.sudoOverlayHit(x, y)
	}
	if s.pendingAsk != nil {
		return s.askOverlayHit(x, y)
	}
	return overlayHit{}, false
}

// handleOverlayMouseClick records the press target of a blocking flyout click.
// Question focus and password refocus move immediately on press (mirroring the
// modal cursor and mention highlight); decisions activate on stationary release.
func (s *session) handleOverlayMouseClick(msg tea.MouseClickMsg) tea.Cmd {
	msg.X, msg.Y = mouseCoord(msg.X, msg.Y)
	if msg.Button != tea.MouseLeft {
		return nil
	}
	hit, ok := s.overlayHitAt(msg.X, msg.Y)
	if !ok {
		s.overlayPress = overlayPress{}
		s.reuseLastView = true
		return nil
	}
	s.overlayPress = overlayPress{active: true, hit: hit, x: msg.X, y: msg.Y}
	if hit.action == overlayAskQuestion && s.pendingAsk != nil &&
		hit.index >= 0 && hit.index < len(s.pendingAsk.questions) {
		s.pendingAsk.jumpToQuestion(hit.index)
	}
	if hit.action == overlaySudoPassword && s.pendingSudo != nil {
		s.pendingSudo.input.Focus()
	}
	return nil
}

// handleOverlayMouseRelease activates an overlay press when the pointer stayed
// on the same target; a drag cancels it without touching the hidden UI.
func (s *session) handleOverlayMouseRelease(msg tea.MouseReleaseMsg) tea.Cmd {
	msg.X, msg.Y = mouseCoord(msg.X, msg.Y)
	p := s.overlayPress
	s.overlayPress = overlayPress{}
	if !p.active || p.dragged {
		s.reuseLastView = true
		return nil
	}
	hit, ok := s.overlayHitAt(msg.X, msg.Y)
	if !ok || hit.action != p.hit.action || (p.hit.index >= 0 && hit.index != p.hit.index) {
		s.reuseLastView = true
		return nil
	}
	return s.activateOverlayHit(hit)
}

// activateOverlayHit fires the stationary-release action of a flyout click.
// Decision actions synthesize the bound send/close keys through the real key
// handlers, so validation and side effects are identical to keypresses.
func (s *session) activateOverlayHit(hit overlayHit) tea.Cmd {
	switch hit.action {
	case overlayAskSubmit:
		return s.dispatchKeyAction("send")
	case overlayAskSkip:
		return s.dispatchKeyAction("close")
	case overlaySudoApprove:
		return s.dispatchKeyAction("send")
	case overlaySudoDecline:
		return s.dispatchKeyAction("close")
	case overlayAskCycleLeft, overlayAskCycleRight:
		return s.cycleAskSelect(hit.index, hit.action == overlayAskCycleRight)
	case overlayAskQuestion, overlaySudoPassword:
		// Focus already moved on press.
		return nil
	}
	return nil
}

// cycleAskSelect moves the inline select of question qIdx one step, mirroring
// the ←/→ keyboard path so huh's private cursor and the bound value stay in
// sync (and an allowCustom "Custom…" option still enters its text input).
func (s *session) cycleAskSelect(qIdx int, forward bool) tea.Cmd {
	a := s.pendingAsk
	if a == nil || a.form == nil || qIdx < 0 || qIdx >= len(a.questions) ||
		a.questions[qIdx].qtype != "select" {
		return nil
	}
	if a.focusedQuestionIndex() != qIdx {
		a.jumpToQuestion(qIdx)
	}
	if a.focusedOnCustom() {
		a.advanceField(-1)
	}
	msg := tea.KeyPressMsg{Code: tea.KeyLeft}
	if forward {
		msg = tea.KeyPressMsg{Code: tea.KeyRight}
	}
	cmd := s.pumpHuhAsk(msg)
	if a.questions[qIdx].allowCustom && a.fieldValues[qIdx] == askCustomSentinel {
		a.advanceField(1)
	}
	return cmd
}

// askOverlayHit maps a screen cell to a clickable ask-flyout target: the
// footer submit/skip affordances, question blocks (focus), and the ‹ › arrows
// of an inline select (cycle options).
func (s *session) askOverlayHit(x, y int) (overlayHit, bool) {
	a := s.pendingAsk
	if a == nil || len(a.questions) == 0 {
		return overlayHit{}, false
	}
	top, left, w, h, rows := s.askOverlayGeom()
	row, col, ok := s.overlayBoxCell(x, y, top, left, w, h)
	if !ok || row >= len(rows) {
		return overlayHit{}, false
	}
	line := rows[row]

	// Footer submit/skip affordances ("[Enter] submit · [Esc] skip"). The key
	// hints come from the live keymap so /keybinds cannot make the targets lie.
	if hit, found := s.askFooterHit(line, col); found {
		return hit, true
	}
	// Question blocks: click focuses the question under the pointer; the select
	// row's ‹ › glyphs cycle its options.
	for _, sp := range s.askQuestionSpans(rows) {
		if row < sp.startRow || row >= sp.endRow {
			continue
		}
		if sp.qtype == "select" && row == sp.valueRow {
			for _, g := range []struct {
				glyph  string
				action overlayClickAction
			}{{"←", overlayAskCycleLeft}, {"→", overlayAskCycleRight}} {
				if i := strings.Index(line, g.glyph); i >= 0 {
					x0 := lipgloss.Width(line[:i])
					if col >= x0 && col < x0+2 {
						return overlayHit{action: g.action, index: sp.index}, true
					}
				}
			}
		}
		return overlayHit{action: overlayAskQuestion, index: sp.index}, true
	}
	return overlayHit{}, false
}

// askFooterHit zones the "[send] submit" and "[close] skip" tokens on the
// flyout's footer row. col is box-relative (the row includes border+padding).
func (s *session) askFooterHit(line string, col int) (overlayHit, bool) {
	sendKey, closeKey := s.keyHint("send"), s.keyHint("close")
	if sendKey == "" || closeKey == "" {
		return overlayHit{}, false
	}
	for _, c := range []struct {
		token  string
		action overlayClickAction
	}{{"[" + sendKey + "] submit", overlayAskSubmit}, {"[" + closeKey + "] skip", overlayAskSkip}} {
		if i := strings.Index(line, c.token); i >= 0 {
			x0 := lipgloss.Width(line[:i])
			if col >= x0 && col < x0+lipgloss.Width(c.token) {
				return overlayHit{action: c.action, index: -1}, true
			}
		}
	}
	return overlayHit{}, false
}

// askQuestionSpan is the rendered row range of one ask question.
type askQuestionSpan struct {
	index    int
	qtype    string
	startRow int // inclusive
	endRow   int // exclusive
	valueRow int // select arrow row / text input row, -1 when not located
}

// askQuestionSpans maps rendered box rows to questions in render order. The
// primary signal is the prompt text in each question's title row; value/input
// rows are located per type (select rows carry ←/→, text rows start with ">").
// A missing title (truncated prompt) falls back to the value row, so a click
// still lands on the right question even on a very narrow terminal.
func (s *session) askQuestionSpans(rows []string) []askQuestionSpan {
	a := s.pendingAsk
	if a == nil || len(a.questions) == 0 {
		return nil
	}
	n := len(a.questions)
	// The footer row is the last row carrying the submit hint; everything above
	// it (up to the box's top border) is question/body territory.
	footerRow := len(rows) - 1
	for i := len(rows) - 2; i >= 0; i-- {
		if strings.Contains(rows[i], "] submit") {
			footerRow = i
			break
		}
	}
	// Title rows by prompt text, scanned forward so overlapping prompts stay
	// on their own question.
	titles := make([]int, n)
	valueRows := make([]int, n)
	for i := range titles {
		titles[i], valueRows[i] = -1, -1
	}
	nextRow := 3 // panel row 0 = title, 1 = blank, body starts at row 2 → box row 3
	for i, q := range a.questions {
		if q.prompt == "" {
			continue
		}
		for r := nextRow; r < footerRow; r++ {
			if strings.Contains(rows[r], q.prompt) {
				titles[i] = r
				nextRow = r + 1
				break
			}
		}
	}
	// Value/input rows, classified by type in render order.
	qIdx := 0
	for r := 3; r < footerRow; r++ {
		trimmed := strings.TrimSpace(rows[r])
		switch {
		case strings.Contains(trimmed, "←") || strings.Contains(trimmed, "→"):
			for qIdx < n && a.questions[qIdx].qtype != "select" {
				qIdx++
			}
			if qIdx < n {
				valueRows[qIdx] = r
				qIdx++
			}
		case strings.HasPrefix(trimmed, ">"):
			for qIdx < n && a.questions[qIdx].qtype == "select" {
				qIdx++
			}
			if qIdx < n {
				valueRows[qIdx] = r
				qIdx++
			}
		}
	}
	spans := make([]askQuestionSpan, n)
	for i := 0; i < n; i++ {
		start := titles[i]
		if start < 0 {
			start = valueRows[i]
		}
		if start < 0 {
			start = 3
		}
		end := footerRow
		for j := i + 1; j < n; j++ {
			if titles[j] >= 0 {
				end = titles[j]
				break
			}
			if valueRows[j] >= 0 {
				end = valueRows[j]
				break
			}
		}
		spans[i] = askQuestionSpan{
			index:    i,
			qtype:    a.questions[i].qtype,
			startRow: start,
			endRow:   end,
			valueRow: valueRows[i],
		}
	}
	return spans
}

// sudoOverlayHit maps a screen cell to a clickable sudo-flyout target: the
// footer approve/decline affordances and the password field region.
func (s *session) sudoOverlayHit(x, y int) (overlayHit, bool) {
	p := s.pendingSudo
	if p == nil {
		return overlayHit{}, false
	}
	top, left, w, h, rows := s.sudoOverlayGeom()
	row, col, ok := s.overlayBoxCell(x, y, top, left, w, h)
	if !ok || row >= len(rows) {
		return overlayHit{}, false
	}
	line := rows[row]
	sendKey, closeKey := s.keyHint("send"), s.keyHint("close")
	if sendKey != "" && closeKey != "" {
		for _, c := range []struct {
			token  string
			action overlayClickAction
		}{{"[" + sendKey + "] approve", overlaySudoApprove}, {"[" + closeKey + "] decline", overlaySudoDecline}} {
			if i := strings.Index(line, c.token); i >= 0 {
				x0 := lipgloss.Width(line[:i])
				if col >= x0 && col < x0+lipgloss.Width(c.token) {
					return overlayHit{action: c.action, index: -1}, true
				}
			}
		}
	}
	// The password label + masked field rows refocus the input. The placeholder
	// guard keeps a zero-value prompt (synthetic test state) from matching.
	if strings.Contains(line, "password") ||
		(p.input.Placeholder != "" && strings.Contains(line, p.input.Placeholder)) {
		return overlayHit{action: overlaySudoPassword, index: -1}, true
	}
	return overlayHit{}, false
}

// chromeLayout mirrors the exact part order assembled in View(). Rows are
// measured with the same render functions View uses (they are cheap chrome
// strings, never a transcript scan), so heights agree with what is painted.
type chromeLayout struct {
	headerRows   int
	failureTop   int
	failureRows  int
	updateTop    int
	updateRows   int
	oauthTop     int
	oauthRows    int
	viewportTop  int
	positionTop  int
	positionRows int
	shelfTop     int
	shelfRows    int
	goalTop      int
	goalRows     int
	mentionTop   int
	mentionRows  int
	inputTop     int
	inputRows    int
	footerTop    int
	footerRows   int
}

func (s *session) chromeLayoutFor() chromeLayout {
	var lay chromeLayout
	lay.headerRows = s.headerHeight()
	y := lay.headerRows
	if b := s.renderCoreFailureBanner(); b != "" {
		lay.failureTop, lay.failureRows = y, lipgloss.Height(b)
		y += lay.failureRows
	}
	if b := s.renderUpdateBanner(); b != "" && s.height >= 10 {
		lay.updateTop, lay.updateRows = y, lipgloss.Height(b)
		y += lay.updateRows
	}
	if b := s.renderOauthBanner(); b != "" {
		lay.oauthTop, lay.oauthRows = y, lipgloss.Height(b)
		y += lay.oauthRows
	}
	lay.viewportTop = y
	y += s.viewport.Height()
	if p := s.renderPositionBar(); p != "" {
		lay.positionTop, lay.positionRows = y, lipgloss.Height(p)
		y += lay.positionRows
	}
	if sh := s.renderActivityShelf(); sh != "" {
		lay.shelfTop, lay.shelfRows = y, lipgloss.Height(sh)
		y += lay.shelfRows
	}
	if gp := s.renderGoalProgressPanel(s.width); gp != "" {
		lay.goalTop, lay.goalRows = y, lipgloss.Height(gp)
		y += lay.goalRows
	}
	if mf := s.renderMentionFlyout(); mf != "" {
		lay.mentionTop, lay.mentionRows = y, lipgloss.Height(mf)
		y += lay.mentionRows
	}
	if wv := s.renderWorkingWave(); wv != "" {
		y += lipgloss.Height(wv)
	}
	if ib := s.renderInputBox(); ib != "" {
		lay.inputTop, lay.inputRows = y, lipgloss.Height(ib)
		y += lay.inputRows
	}
	if ft := s.renderFooter(); ft != "" {
		lay.footerTop, lay.footerRows = y, lipgloss.Height(ft)
	}
	return lay
}

// chromeHitAt maps a pointer cell to a chrome target, or chromeNone when the
// pointer is over the transcript viewport (selection territory) or an inert
// chrome row (working wave, diff preview…). Update banners are actionable and
// are mapped to chromeUpdate.
func (s *session) chromeHitAt(x, y int) chromeHit {
	lay := s.chromeLayoutFor()
	if y < 0 {
		return chromeHit{}
	}
	switch {
	case y < lay.headerRows:
		return s.headerModelHit(x, lay)
	case lay.failureRows > 0 && y >= lay.failureTop && y < lay.failureTop+lay.failureRows:
		return s.coreFailureHit(x, y, lay)
	case lay.updateRows > 0 && y >= lay.updateTop && y < lay.updateTop+lay.updateRows:
		return chromeHit{zone: chromeUpdate, action: -1}
	case lay.oauthRows > 0 && y >= lay.oauthTop && y < lay.oauthTop+lay.oauthRows:
		return chromeHit{zone: chromeOauth, action: -1}
	case lay.positionRows > 0 && y >= lay.positionTop && y < lay.positionTop+lay.positionRows:
		return chromeHit{zone: chromePositionBar, action: -1}
	case lay.shelfRows > 0 && y >= lay.shelfTop && y < lay.shelfTop+lay.shelfRows:
		return s.shelfHit(x, y, lay)
	case lay.goalRows > 0 && y >= lay.goalTop && y < lay.goalTop+lay.goalRows:
		return chromeHit{zone: chromeGoalPanel, action: -1}
	case lay.mentionRows > 0 && y >= lay.mentionTop && y < lay.mentionTop+lay.mentionRows:
		return s.mentionHit(x, y, lay)
	case lay.inputRows > 0 && y >= lay.inputTop && y < lay.inputTop+lay.inputRows:
		return s.inputHit(x, y, lay)
	case lay.footerRows > 0 && y >= lay.footerTop && y < lay.footerTop+lay.footerRows:
		return s.footerHit(x, y, lay)
	}
	return chromeHit{}
}

// headerModelHit zones the " · <model>" label on the right side of the header
// so a click opens the model picker.
func (s *session) headerModelHit(x int, lay chromeLayout) chromeHit {
	if s.width < 42 || len(s.models) == 0 || s.modelIdx < 0 || s.modelIdx >= len(s.models) {
		return chromeHit{}
	}
	plain := stripANSI(s.renderHeader())
	tok := " · " + truncate(s.models[s.modelIdx].ID, max(8, s.width/3))
	if i := strings.LastIndex(plain, tok); i >= 0 {
		x0 := lipgloss.Width(plain[:i])
		x1 := x0 + lipgloss.Width(tok)
		if x >= x0 && x < x1 {
			return chromeHit{zone: chromeHeaderModel, action: -1}
		}
	}
	return chromeHit{}
}

// coreFailureHit zones the "r retry" and "q quit" hints of the recovery
// banner (the banner itself renders those literal hints, not keybinds).
func (s *session) coreFailureHit(x, y int, lay chromeLayout) chromeHit {
	rows := plainTranscriptLines(s.renderCoreFailureBanner())
	row := y - lay.failureTop
	if row < 0 || row >= len(rows) {
		return chromeHit{}
	}
	line := rows[row]
	// Only the fixed suffix is actionable. Core error text is untrusted and may
	// itself contain words such as "r retry" or "q quit".
	suffix := "r retry · q quit"
	i := strings.LastIndex(line, suffix)
	if i < 0 {
		return chromeHit{}
	}
	suffixLine := line[i:]
	for _, c := range []struct {
		token  string
		action int
	}{{"r retry", coreFailureRetry}, {"q quit", coreFailureQuit}} {
		if j := strings.Index(suffixLine, c.token); j >= 0 {
			x0 := lipgloss.Width(line[:i+j])
			x1 := x0 + lipgloss.Width(c.token)
			if x >= x0 && x < x1 {
				return chromeHit{zone: chromeCoreFailure, action: c.action}
			}
		}
	}
	return chromeHit{}
}

// shelfHit dispatches by content: decision banners (approval/intercom/queue)
// get their own zones, the goal-only label toggles the goal panel, and the
// routine activity shelf toggles between collapsed and expanded.
func (s *session) shelfHit(x, y int, lay chromeLayout) chromeHit {
	row := y - lay.shelfTop
	switch {
	case s.pendingApproval != nil:
		return s.approvalShelfHit(x, row)
	case s.pendingIntercom != nil:
		return s.intercomShelfHit(x, row)
	case s.queued != nil:
		return s.queueShelfHit(x, row)
	case len(s.todos) == 0 && len(s.subProgress) == 0 &&
		s.goalState != nil && goalShowsProgressPanel(s.goalState.Phase, s.goalState.AutoDeploy):
		// The goal-only label is a duplicate of the goal panel header below;
		// clicking it toggles the panel (collapsed row → expand, and vice
		// versa) instead of expanding a shelf with nothing to show.
		return chromeHit{zone: chromeShelf, action: shelfActionGoalLabel}
	}
	rows := plainTranscriptLines(s.renderActivityShelf())
	if row < 0 || row >= len(rows) {
		return chromeHit{}
	}
	if !s.activityExpanded {
		return chromeHit{zone: chromeShelf, action: shelfActionToggle}
	}
	// Expanded panel: the header (border + "Activity …" rows up to the first
	// section marker) toggles back to the collapsed row; detail rows are not
	// individually actionable.
	headerRows := 0
	for _, ln := range rows {
		trimmed := strings.TrimSpace(ln)
		if strings.HasPrefix(trimmed, "Subagents") || strings.HasPrefix(trimmed, "Tasks") || strings.HasPrefix(trimmed, "╰") {
			break
		}
		headerRows++
	}
	if row < headerRows {
		return chromeHit{zone: chromeShelf, action: shelfActionToggle}
	}
	return chromeHit{}
}

// queueShelfHit only exposes the rendered cancel hint. The queued prompt text
// is untrusted and may contain words that look like controls.
func (s *session) queueShelfHit(x, row int) chromeHit {
	rows := plainTranscriptLines(s.renderQueueBanner())
	if row < 0 || row >= len(rows) {
		return chromeHit{}
	}
	key := s.keyHint("close")
	if key == "" {
		return chromeHit{}
	}
	line := rows[row]
	token := key + " to cancel"
	if i := strings.LastIndex(line, token); i >= 0 {
		x0 := lipgloss.Width(line[:i])
		if x >= x0 && x < x0+lipgloss.Width(token) {
			return chromeHit{zone: chromeShelf, action: shelfActionQueueCancel}
		}
	}
	// The entire queue banner is a cancellation affordance. This keeps the
	// target useful when the hint is hidden by a narrow-width clamp or a custom
	// close binding that cannot be represented in the rendered line.
	return chromeHit{zone: chromeShelf, action: shelfActionQueueCancel}
}

// approvalShelfHit zones the "[Y] once · [N] deny · [A] type" controls of the
// approval banner using the live key hints, so /keybinds never makes the
// click targets lie. The banner can wrap at narrow widths, so tokens are
// matched per rendered row.
func (s *session) approvalShelfHit(x, row int) chromeHit {
	rows := plainTranscriptLines(s.renderApprovalBanner())
	if row < 0 || row >= len(rows) {
		return chromeHit{}
	}
	line := rows[row]
	for _, c := range []struct {
		keyAction string
		desc      string
		action    int
	}{
		{"approve", "once", shelfActionApprovalOnce},
		{"deny", "deny", shelfActionApprovalDeny},
		{"approve_always", "type", shelfActionApprovalAlways},
	} {
		key := s.keyHint(c.keyAction)
		if key == "" {
			continue
		}
		if i := strings.LastIndex(line, "["+key+"] "+c.desc); i >= 0 {
			x0 := lipgloss.Width(line[:i])
			x1 := x0 + lipgloss.Width("["+key+"] "+c.desc)
			if x >= x0 && x < x1 {
				return chromeHit{zone: chromeShelf, action: c.action}
			}
		}
	}
	return chromeHit{}
}

// intercomShelfHit zones the "type reply + Enter" submit and "Esc skip" skip
// affordances of the intercom banner (both render formats: the normal ask and
// the post-empty-Enter nudge pulse).
func (s *session) intercomShelfHit(x, row int) chromeHit {
	rows := plainTranscriptLines(s.renderIntercomBanner())
	if row < 0 || row >= len(rows) {
		return chromeHit{}
	}
	line := rows[row]
	// Match only the fixed affordance suffix, not a subagent message that may
	// contain the same words.
	sendKey := s.keyHint("send")
	if sendKey == "" {
		sendKey = "send"
	}
	closeKey := s.keyHint("close")
	if closeKey == "" {
		closeKey = "skip"
	}
	affordance := "type reply + " + sendKey
	if !strings.Contains(line, affordance) {
		affordance = "then " + sendKey
	}
	skipAffordance := closeKey + " skip"
	for _, c := range []struct {
		token  string
		action int
	}{
		{affordance, shelfActionIntercomSubmit},
		{skipAffordance, shelfActionIntercomSkip},
		{closeKey + " to skip", shelfActionIntercomSkip},
	} {
		if i := strings.LastIndex(line, c.token); i >= 0 {
			x0 := lipgloss.Width(line[:i])
			x1 := x0 + lipgloss.Width(c.token)
			if x >= x0 && x < x1 {
				return chromeHit{zone: chromeShelf, action: c.action}
			}
		}
	}
	return chromeHit{}
}

// mentionHit maps a flyout row to the mention item under it. Only real item
// rows are actionable (loading/failed/empty states and the hint row are not).
func (s *session) mentionHit(x, y int, lay chromeLayout) chromeHit {
	if !s.mentionActive {
		return chromeHit{}
	}
	n := len(s.mentionItems)
	if n == 0 {
		return chromeHit{}
	}
	// Mirror the renderer: a non-empty, non-slash query switches the flyout to
	// loading/failed and stops showing item rows.
	state := mentionSearchReady
	runes := []rune(s.input.Value())
	pos := inputPosition(s.input)
	if s.mentionAt >= 0 && pos > s.mentionAt && pos <= len(runes) {
		query := string(runes[s.mentionAt+1 : pos])
		if query != "" && !strings.Contains(query, "/") {
			state, _ = currentMentionSearchState()
		}
	}
	if state != mentionSearchReady {
		return chromeHit{}
	}
	row := y - lay.mentionTop - 1 // first body row is one below the top border
	if row < 0 {
		return chromeHit{}
	}
	end := min(n, s.mentionScroll+mentionMaxVisible)
	if row >= end-s.mentionScroll {
		return chromeHit{} // hint row
	}
	idx := s.mentionScroll + row
	if idx < 0 || idx >= n {
		return chromeHit{}
	}
	return chromeHit{zone: chromeMention, action: idx}
}

// footerControl pairs a rendered footer hint description with the keybind
// action it represents (index into footerControlDefs).
type footerControl struct {
	index  int
	x0, x1 int
}

// footerControlDefs lists the footer help descriptions in display order.
// ShortHelpView renders "key desc" pairs; the description tokens are unique
// per control mode (idle/busy/approval), so a plain-text scan is precise.
var footerControlDefs = []struct {
	desc   string
	action string
}{
	{"send", "send"},
	{"newline", "newline"},
	{"commands", "command_palette"},
	{"queue", "send"},
	{"abort", "close"},
	{"steer", "steer"},
	{"allow once", "approve"},
	{"deny", "deny"},
	{"always allow type", "approve_always"},
}

// footerControls finds the clickable control spans in the footer's first
// (control) line. Returns them left-to-right.
func footerControls(line string) []footerControl {
	return footerControlsFor(line, footerControlDefs)
}

func footerControlsFor(line string, defs []struct {
	desc   string
	action string
}) []footerControl {
	var out []footerControl
	for i, c := range defs {
		if j := strings.Index(line, " "+c.desc); j >= 0 {
			x0 := lipgloss.Width(line[:j+1])
			idx := i
			for j := range footerControlDefs {
				if footerControlDefs[j].action == c.action && footerControlDefs[j].desc == c.desc {
					idx = j
					break
				}
			}
			out = append(out, footerControl{index: idx, x0: x0, x1: x0 + lipgloss.Width(c.desc)})
		}
	}
	sort.Slice(out, func(a, b int) bool { return out[a].x0 < out[b].x0 })
	return out
}

// footerHit zones the primary controls on the footer's first line. The second
// line (perf metrics) and toast replacement rows have no controls.
func (s *session) footerHit(x, y int, lay chromeLayout) chromeHit {
	if y != lay.footerTop {
		return chromeHit{}
	}
	// A toast replaces the visible control rail. Do not let words in the toast
	// become accidental buttons.
	if s.renderToast() != "" {
		return chromeHit{}
	}
	first := strings.SplitN(stripANSI(s.renderFooter()), "\n", 2)[0]
	defs := footerControlDefs
	switch {
	case s.pendingApproval != nil:
		defs = footerControlDefs[6:]
	case s.busy:
		defs = footerControlDefs[3:6]
	default:
		defs = footerControlDefs[:3]
	}
	for _, c := range footerControlsFor(first, defs) {
		if x >= c.x0 && x < c.x1 {
			return chromeHit{zone: chromeFooter, action: c.index}
		}
	}
	return chromeHit{}
}

// inputHit keeps the composer card clickable while giving its visible image
// removal affordance a more specific target than ordinary text placement.
func (s *session) inputHit(x, y int, lay chromeLayout) chromeHit {
	if len(s.pendingImages) > 0 && y == lay.inputTop+1 {
		lines := strings.Split(stripANSI(s.renderInputBox()), "\n")
		row := y - lay.inputTop - 1
		if row >= 0 && row < len(lines) {
			if key := s.keyHint("detach_image"); key != "" {
				token := key + " remove"
				if i := strings.LastIndex(lines[row], token); i >= 0 {
					x0 := lipgloss.Width(lines[row][:i])
					if x >= x0 && x < x0+lipgloss.Width(token) {
						return chromeHit{zone: chromeInputBox, action: chromeInputDetach}
					}
				}
			}
		}
	}
	return chromeHit{zone: chromeInputBox, action: chromeInputFocus}
}

// chromePressSideEffects applies immediate press-time effects (mention cursor
// highlight, composer focus) so the UI reacts on press, not release.
func (s *session) chromePressSideEffects(hit chromeHit) {
	switch hit.zone {
	case chromeMention:
		if hit.action >= 0 && hit.action < len(s.mentionItems) {
			s.mentionCursor = hit.action
		}
	case chromeInputBox:
		s.input.Focus()
	}
}

// activateChromeHit fires the action for a stationary press+release on the
// same target. Actions reuse the existing key-handling paths (handleKey with
// the bound key) instead of duplicating their logic; only the approval
// empty-composer guard is enforced here because a synthesized "y" would
// otherwise type into the draft.
func (s *session) activateChromeHit(hit chromeHit, x, y int) tea.Cmd {
	switch hit.zone {
	case chromeHeaderModel:
		s.openModelPicker()
		return nil
	case chromeCoreFailure:
		if hit.action == coreFailureRetry {
			s.resetCoreUIState()
			return s.startCore()
		}
		if hit.action == coreFailureQuit {
			return s.quit()
		}
	case chromeUpdate:
		if s.updating {
			return nil
		}
		s.updating = true
		return s.updateBannerClick()
	case chromeOauth:
		return s.oauthBannerClick()
	case chromePositionBar:
		s.follow = true
		s.viewport.GotoBottom()
		return nil
	case chromeShelf:
		return s.shelfAction(hit.action)
	case chromeGoalPanel:
		if s.goalState != nil && goalShowsProgressPanel(s.goalState.Phase, s.goalState.AutoDeploy) {
			s.goalPanelCollapsed = !s.goalPanelCollapsed
			s.layout()
		}
		return nil
	case chromeMention:
		if hit.action >= 0 && hit.action < len(s.mentionItems) {
			s.mentionCursor = hit.action
			s.acceptMention()
		}
		return nil
	case chromeInputBox:
		if hit.action == chromeInputDetach {
			return s.dispatchKeyAction("detach_image")
		}
		if pos, ok := s.composerClickCursor(x, y); ok {
			setInputCursor(&s.input, pos)
		}
		return nil
	case chromeFooter:
		return s.footerAction(hit.action)
	}
	return nil
}

func (s *session) shelfAction(act int) tea.Cmd {
	switch act {
	case shelfActionGoalLabel:
		if s.goalState != nil && goalShowsProgressPanel(s.goalState.Phase, s.goalState.AutoDeploy) {
			s.goalPanelCollapsed = !s.goalPanelCollapsed
			s.layout()
		}
		return nil
	case shelfActionQueueCancel:
		return s.cancelQueued()
	case shelfActionApprovalOnce, shelfActionApprovalDeny, shelfActionApprovalAlways:
		if s.pendingApproval == nil {
			return nil
		}
		if strings.TrimSpace(s.input.Value()) != "" {
			s.setToast(toastWarn, "clear input to answer first")
			return nil
		}
		action := ""
		switch act {
		case shelfActionApprovalOnce:
			action = "approve"
		case shelfActionApprovalDeny:
			action = "deny"
		case shelfActionApprovalAlways:
			action = "approve_always"
		}
		return s.dispatchKeyAction(action)
	case shelfActionIntercomSubmit, shelfActionIntercomSkip:
		if s.pendingIntercom == nil {
			return nil
		}
		if act == shelfActionIntercomSkip {
			return s.dispatchKeyAction("close")
		}
		return s.dispatchKeyAction("send")
	case shelfActionToggle:
		if s.activityExpanded {
			s.activityExpanded = false
			s.activityScroll = 0
		} else {
			s.activityExpanded = true
		}
		s.layout()
		return nil
	}
	return nil
}

// cancelQueued mirrors the keyboard Esc-on-queued path: drop the queued
// follow-up/steer without aborting the in-flight turn.
func (s *session) cancelQueued() tea.Cmd {
	if s.queued == nil {
		return nil
	}
	kind := s.queued.kind
	s.queued = nil
	s.queuedNext = false
	s.sendCore(map[string]any{"type": "clear_queue"})
	s.layout()
	if kind == "steer" {
		s.logInfo("steer cancelled (the running turn was already interrupted)")
	} else {
		s.logInfo("queued follow-up cancelled — turn continues")
	}
	return nil
}

// oauthBannerClick re-copies the OAuth URL (and code when present) to the
// clipboard. The URL is only re-opened in the browser when it is a safe
// https/localhost target; anything else stays copy-only.
func (s *session) oauthBannerClick() tea.Cmd {
	o := s.oauth
	if o == nil {
		return nil
	}
	var cmds []tea.Cmd
	switch {
	case o.url != "":
		cmds = append(cmds, tea.SetClipboard(o.url), tea.SetPrimaryClipboard(o.url))
		if isSafeOauthURL(o.url) {
			openURL(o.url)
			s.setToast(toastSuccess, "OAuth URL copied + opened")
		} else {
			s.setToast(toastSuccess, "OAuth URL copied (browser open skipped)")
		}
	case o.code != "":
		cmds = append(cmds, tea.SetClipboard(o.code), tea.SetPrimaryClipboard(o.code))
		s.setToast(toastSuccess, "OAuth code copied")
	default:
		return nil
	}
	return tea.Batch(cmds...)
}

// isSafeOauthURL reports whether a URL is safe to hand to the OS browser
// opener. Only https (any host) and http loopback targets qualify; anything
// else stays copy-only. Credentials in the URL are rejected outright — OAuth
// URLs never carry userinfo, and browsers render user@host confusingly.
func isSafeOauthURL(u string) bool {
	u = strings.TrimSpace(u)
	if u == "" {
		return false
	}
	p, err := url.Parse(u)
	if err != nil || p.Hostname() == "" || p.User != nil {
		return false
	}
	if strings.EqualFold(p.Scheme, "https") {
		return true
	}
	if !strings.EqualFold(p.Scheme, "http") {
		return false
	}
	switch strings.ToLower(p.Hostname()) {
	case "localhost", "127.0.0.1", "::1":
		return true
	default:
		return false
	}
}

// updateBannerClick runs the same signed self-updater exposed by
// `catcode --update`, under Bubble Tea's ExecProcess so the terminal is
// restored while the updater writes progress. A second click while an update is
// already running is ignored: both would replace the executable concurrently.
func (s *session) updateBannerClick() tea.Cmd {
	if s.updateInfo == nil {
		return nil
	}
	if s.updating {
		s.setToast(toastInfo, "update already in progress…")
		return nil
	}
	exe, err := os.Executable()
	if err != nil || exe == "" {
		s.setToast(toastError, "could not locate catcode for update")
		return nil
	}
	s.updating = true
	s.setToast(toastInfo, "updating catcode…")
	cmd := exec.Command(exe, "--update")
	return tea.ExecProcess(cmd, func(err error) tea.Msg {
		return updateExecMsg{err: err}
	})
}

// footerAction maps a clicked footer control to its keybind action. Approval
// decisions respect the empty-composer guard so a synthesized "y" never types
// into a draft; everything else reuses the exact keyboard handler.
func (s *session) footerAction(idx int) tea.Cmd {
	if idx < 0 || idx >= len(footerControlDefs) {
		return nil
	}
	action := footerControlDefs[idx].action
	if action == "approve" || action == "deny" || action == "approve_always" {
		if strings.TrimSpace(s.input.Value()) != "" {
			s.setToast(toastWarn, "clear input to answer first")
			return nil
		}
	}
	return s.dispatchKeyAction(action)
}

// dispatchKeyAction synthesizes the key bound to action and routes it through
// the real key handler, so mouse and keyboard share every decision path.
func (s *session) dispatchKeyAction(action string) tea.Cmd {
	msg, ok := s.keyPressFor(action)
	if !ok {
		return nil
	}
	_, cmd := s.handleKey(msg)
	return cmd
}

// keyPressFor converts a canonical keymap string ("enter", "ctrl+p",
// "shift+enter", "y", …) back into the KeyPressMsg the keyboard path expects.
func (s *session) keyPressFor(action string) (tea.KeyPressMsg, bool) {
	key := s.keybinds[action]
	if key == "" {
		return tea.KeyPressMsg{}, false
	}
	parts := strings.Split(key, "+")
	var mod tea.KeyMod
	for _, p := range parts[:len(parts)-1] {
		p = strings.ToLower(p)
		switch p {
		case "ctrl":
			mod |= tea.ModCtrl
		case "shift":
			mod |= tea.ModShift
		case "alt":
			mod |= tea.ModAlt
		case "meta":
			mod |= tea.ModMeta
		case "hyper":
			mod |= tea.ModHyper
		case "super":
			mod |= tea.ModSuper
		default:
			return tea.KeyPressMsg{}, false
		}
	}
	msg := tea.KeyPressMsg{Mod: mod}
	name := strings.ToLower(strings.TrimSpace(parts[len(parts)-1]))
	switch name {
	case "enter", "return":
		msg.Code = tea.KeyEnter
	case "esc", "escape":
		msg.Code = tea.KeyEscape
	case "tab":
		msg.Code = tea.KeyTab
	case "space":
		msg.Code = tea.KeySpace
	case "up":
		msg.Code = tea.KeyUp
	case "down":
		msg.Code = tea.KeyDown
	case "left":
		msg.Code = tea.KeyLeft
	case "right":
		msg.Code = tea.KeyRight
	case "pgup":
		msg.Code = tea.KeyPgUp
	case "pgdown":
		msg.Code = tea.KeyPgDown
	case "home":
		msg.Code = tea.KeyHome
	case "end":
		msg.Code = tea.KeyEnd
	case "backspace":
		msg.Code = tea.KeyBackspace
	case "delete":
		msg.Code = tea.KeyDelete
	case "insert":
		msg.Code = tea.KeyInsert
	case "select":
		msg.Code = tea.KeySelect
	case "begin":
		msg.Code = tea.KeyBegin
	case "find":
		msg.Code = tea.KeyFind
	default:
		if n, err := strconv.Atoi(strings.TrimPrefix(name, "f")); strings.HasPrefix(name, "f") && err == nil && n >= 1 && n <= 63 {
			msg.Code = tea.KeyF1 + rune(n-1)
			return msg, true
		}
		r := []rune(parts[len(parts)-1])
		if len(r) != 1 {
			return tea.KeyPressMsg{}, false
		}
		msg.Code = r[0]
	}
	return msg, true
}

// composerClickCursor maps a click inside the composer card to an absolute
// rune offset for the textarea cursor, mirroring inputContent's wrapping,
// chip/hint rows, and maxInputLines windowing so the cursor lands on the same
// visible text the renderer drew.
func (s *session) composerClickCursor(x, y int) (int, bool) {
	if s.width < 8 {
		return inputPosition(s.input), true
	}
	lay := s.chromeLayoutFor()
	textW := max(1, s.width-6)
	value := s.input.Value()
	r := []rune(value)
	row := y - lay.inputTop - 1 // first card row is the top border
	if row < 0 {
		return 0, false
	}
	if len(s.pendingImages) > 0 {
		row-- // attachment chip row sits above the text
		if row < 0 {
			return 0, false
		}
	}
	// inputContent renders a cursor line rather than a plain soft-wrap. Build
	// the same display rows so a click at a wrap boundary follows the painted
	// cursor/text instead of the raw wrap approximation.
	pos := inputPosition(s.input)
	pos = min(max(0, pos), len(r))
	beforeLines := wrapRunesMultiline(r[:pos], textW)
	cLine := len(beforeLines) - 1
	cCol := len(beforeLines[cLine])
	if cCol >= textW {
		cLine++
	}
	lines := wrapRunesMultiline(r, textW)
	hasHint := value != "" && s.composerHintLine(max(1, s.width-4)) != ""
	if row >= len(lines) && row != cLine {
		if hasHint && row == len(lines) {
			return len(r), true
		}
		return 0, false // bottom border or beyond
	}
	// maxInputLines windowing mirrors inputContent: rows are centered on the
	// cursor line with "…" markers at the edges.
	if len(lines) > maxInputLines || cLine >= maxInputLines {
		pos := inputPosition(s.input)
		pos = min(max(0, pos), len(r))
		before := r[:pos]
		beforeLines := wrapRunesMultiline(before, textW)
		cLine := len(beforeLines) - 1
		if cLine >= 0 && len(beforeLines[cLine]) >= textW {
			cLine++ // cursor exactly at a wrap boundary sits on a fresh line
		}
		half := maxInputLines / 2
		start := cLine - half
		if start < 0 {
			start = 0
		}
		end := start + maxInputLines
		if end > len(lines) {
			end = len(lines)
			start = max(0, end-maxInputLines)
		}
		vr := row
		if start > 0 {
			vr-- // skip the top "…" marker
		}
		if vr < 0 {
			return 0, false
		}
		if vr >= end-start {
			return len(r), true // bottom marker / below the visible window
		}
		row = start + vr
	}
	// Map the clicked cell to a rune offset within the row. Text starts at
	// x+4: border(1) + padding(1) + the "❯ "/"  " prefix(2).
	textCol := x - 4
	line := lines[row]
	runesInRow := 0
	cell := 0
	g := uniseg.NewGraphemes(string(line))
	for g.Next() {
		w := max(1, g.Width())
		if textCol <= cell+w/2 {
			break
		}
		runesInRow += len([]rune(g.Str()))
		cell += w
	}
	if textCol > cell {
		runesInRow = len(line)
	}
	return wrappedRuneOffset(r, textW, row) + runesInRow, true
}

// wrappedRuneOffset returns the absolute rune index where display row `row`
// starts, accounting for '\n' segmentation exactly like wrapRunesMultiline.
func wrappedRuneOffset(r []rune, w, row int) int {
	if w < 1 {
		w = 1
	}
	pos := 0
	rowIdx := 0
	for pos <= len(r) {
		segEnd := pos
		for segEnd < len(r) && r[segEnd] != '\n' {
			segEnd++
		}
		segLen := segEnd - pos
		rows := segLen / w
		if segLen%w != 0 {
			rows++
		}
		if segLen == 0 {
			rows = 1 // empty segment renders one empty row
		}
		if row < rowIdx+rows {
			off := pos + (row-rowIdx)*w
			if off > segEnd {
				off = segEnd
			}
			return off
		}
		rowIdx += rows
		if segEnd >= len(r) {
			break
		}
		pos = segEnd + 1
	}
	return len(r)
}
