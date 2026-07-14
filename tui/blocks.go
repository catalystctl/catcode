package main

import (
	"encoding/json"
	"fmt"
	"path/filepath"
	"strings"
	"time"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

// ---------------------------------------------------------------------------
// Block model
//
// The conversation is a list of typed blocks rather than a raw string buffer.
// Streaming blocks (assistant, thinking) accumulate into .text; the rest are
// set once. Finalized blocks are rendered once and cached; the live streaming
// block is re-rendered per token, so per-token cost stays O(1) amortized.
// ---------------------------------------------------------------------------

type blockKind int

const (
	blkUser blockKind = iota
	blkAssistant
	blkThinking
	blkTool
	blkToolResult
	blkInfo
	blkSuccess
	blkWarn
	blkError
	blkApprove
	blkTrimmed // synthetic marker showing that older in-memory history was evicted
	blkRaw     // pre-styled string rendered verbatim (e.g. the /model list)
)

type approvalBlockState uint8

const (
	approvalPending approvalBlockState = iota
	approvalApproved
	approvalAlways
	approvalDenied
)

type block struct {
	kind        blockKind
	text        strings.Builder // streamed content (assistant / thinking) or raw
	name        string          // tool name (blkTool / blkApprove)
	args        string          // tool args
	output      string          // blkToolResult
	diff        string          // blkTool: unified-diff text (tool_result only); shown instead of output when present
	started     time.Time       // blkTool: when the call began
	dur         time.Duration   // blkTool: elapsed until result
	collapsed   bool            // blkThinking only
	model       string          // blkAssistant: model id (for the role line)
	sub         bool            // blkTool: a spawn:* sub-agent internal call (collapsed)
	id          string          // blkTool: tool-call id, to match results out of order
	expanded    bool            // blkTool / blkToolResult: full output shown (ctrl+o)
	ok          bool            // blkTool: outcome.ok from tool_result (false on error/deny)
	hasOk       bool            // blkTool: true once a result landed (distinguishes in-flight)
	renderW     int             // P1-12: width the streaming block was last rendered at
	renderLen   int             // P1-12: text length at last render (throttle)
	renderStr   string          // P1-12: cached render of the streaming block
	renderStart int             // first rendered transcript line (zero based)
	renderEnd   int             // last rendered transcript line (inclusive)
	approval    approvalBlockState
	trimmed     int // blkTrimmed: number of evicted blocks
}

// push appends a block and updates the streaming cursor. Streaming kinds
// (assistant, thinking) become s.cur; everything else finalizes s.cur=nil so
// the previous streaming block gets cached.
// maxBlocks caps the in-memory transcript so a long session doesn't grow
// `blocks` + the pre-rendered `cache` (~3x the transcript) without bound (P1-16).
const maxBlocks = 400

func (s *session) push(kind blockKind) *block {
	b := &block{kind: kind}
	if kind == blkThinking {
		b.collapsed = !s.thinkExpanded
	}
	s.blocks = append(s.blocks, b)
	if len(s.blocks) > maxBlocks {
		// Drop the oldest finalized blocks and reset the render cache (its prefix
		// no longer matches the shifted indices). `s.cur` is always the newest
		// block, so it's never trimmed.
		trim := len(s.blocks) - (maxBlocks - 1)
		trimmed := trim
		if len(s.blocks) > 0 && s.blocks[0] != nil && s.blocks[0].kind == blkTrimmed {
			trim = len(s.blocks) - maxBlocks
			trimmed = trim + s.blocks[0].trimmed
			trim++ // discard the old marker as well
		}
		// Copy into a fresh backing slice instead of slicing (s.blocks[trim:]).
		// Slicing alone keeps the old backing array — and the *block pointers in
		// its dropped prefix — alive for the whole session, pinning the trimmed
		// blocks' rendered strings and slowly creeping RSS. A fresh copy lets the
		// GC reclaim the old array and its stale prefix.
		kept := make([]*block, 0, maxBlocks+8)
		kept = append(kept, &block{kind: blkTrimmed, trimmed: trimmed})
		kept = append(kept, s.blocks[trim:]...)
		s.blocks = kept
		if s.focusedBlock >= 0 {
			s.focusedBlock = s.focusedBlock - trim + 1 // new marker occupies index 0
			if s.focusedBlock < 0 {
				s.focusedBlock = -1
			}
		}
		s.invalidateAll()
	}
	if kind == blkAssistant || kind == blkThinking {
		s.cur = b
	} else {
		s.cur = nil
	}
	return b
}

// invalidateAll drops the render cache (used on resize / collapse-toggle,
// where cached wrapped renders are stale).
func (s *session) invalidateAll() {
	s.cache.Reset()
	s.cacheIdx = 0
	for _, b := range s.blocks {
		if b != nil {
			b.renderStr = ""
		}
	}
	for _, b := range s.blocks {
		if b != nil {
			b.renderStart, b.renderEnd = 0, -1
		}
	}
}

// renderBlocks returns the full viewport content. Finalized blocks are cached
// in s.cache and only extended; the live streaming block (s.cur) and any
// in-flight tool blocks (awaiting their result) are re-rendered each call so
// the active-tasks panel's elapsed time stays live.
// ponytail: full re-render (invalidateAll) only on resize/toggle; very long
// histories re-wrap the current streaming block per token (upgrade: cache
// wrapped lines per finalized block keyed by width).
// hasConversation reports whether the transcript contains real chat turns
// (user/assistant/tools). Status/error-only blocks do not count, so the
// welcome screen survives startup chatter and failed auth probes.
func (s *session) hasConversation() bool {
	for _, b := range s.blocks {
		if b == nil {
			continue
		}
		switch b.kind {
		case blkUser, blkAssistant, blkThinking, blkTool, blkToolResult, blkApprove:
			return true
		}
	}
	return false
}

func (s *session) renderBlocks() string {
	if s.coreLifecycle == coreFailed && !s.hasConversation() {
		return s.renderCoreFailure()
	}
	if !s.hasConversation() {
		return s.renderWelcome()
	}
	w := s.viewport.Width()
	for s.cacheIdx < len(s.blocks) {
		blk := s.blocks[s.cacheIdx]
		if blk == s.cur || isInFlight(blk) {
			break // don't cache the live streaming block or in-flight tools
		}
		start := renderedLineOffset(s.cache.String())
		rendered := s.renderBlock(blk, w)
		blk.renderStart, blk.renderEnd = start, start+lipgloss.Height(rendered)-1
		s.cache.WriteString(rendered)
		s.cache.WriteString("\n\n") // breathing room between blocks
		s.cacheIdx++
	}
	var b strings.Builder
	b.WriteString(s.cache.String())
	if s.cur != nil {
		rendered := s.renderBlock(s.cur, w)
		start := renderedLineOffset(b.String())
		s.cur.renderStart, s.cur.renderEnd = start, start+lipgloss.Height(rendered)-1
		b.WriteString(rendered)
		b.WriteString("\n\n")
	}
	// Trailing in-flight tools render live: a spawn scout is shown in the
	// active-tasks panel (not inline); other tools get a brief head marker
	// until their result lands, then they re-cache as a full card.
	for i := s.cacheIdx; i < len(s.blocks); i++ {
		blk := s.blocks[i]
		if blk == s.cur || !isInFlight(blk) {
			continue
		}
		if blk.name == "spawn" || blk.name == "subagent" {
			continue // active-tasks panel renders the scout
		}
		rendered := s.renderBlock(blk, w)
		start := renderedLineOffset(b.String())
		blk.renderStart, blk.renderEnd = start, start+lipgloss.Height(rendered)-1
		b.WriteString(rendered)
		b.WriteString("\n\n")
	}
	return strings.TrimRight(b.String(), "\n")
}

func renderedLineOffset(s string) int {
	if s == "" {
		return 0
	}
	return strings.Count(s, "\n")
}

// scheduleStreamRefresh coalesces token deltas into one viewport rebuild per
// frame. The core-event pump remains armed, so streaming throughput and tool /
// approval latency are unaffected while long transcripts avoid SetContent on
// every token.
func (s *session) scheduleStreamRefresh() tea.Cmd {
	wait := waitForEvent(s.coreEvents, s.coreStartGen)
	if s.streamRefreshPending {
		return wait
	}
	s.streamRefreshPending = true
	frameDelay := 33 * time.Millisecond
	if len(s.blocks) > 300 {
		frameDelay = 100 * time.Millisecond
	} else if len(s.blocks) > 100 {
		frameDelay = 66 * time.Millisecond
	}
	return tea.Batch(wait, tea.Tick(frameDelay, func(time.Time) tea.Msg {
		return streamRefreshMsg{}
	}))
}

func (s *session) renderCoreFailure() string {
	w, h := s.viewport.Width(), s.viewport.Height()
	msg := strings.TrimSpace(s.coreFailure)
	if msg == "" {
		msg = "The core failed to start."
	}
	rows := []string{
		errStyle.Render("Core unavailable"), "", baseStyle.Render(msg), "",
		accentStyle.Render("r") + baseStyle.Render(" retry   ") + accentStyle.Render("q") + baseStyle.Render(" quit"),
		"", dimStyle.Render("Debug log: " + filepath.Join(configDir(), "debug.jsonl")),
	}
	panelW := min(68, max(20, w-4))
	panel := lipgloss.NewStyle().BorderStyle(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(c.err)).Padding(0, 1).Width(panelW).
		Render(strings.Join(rows, "\n"))
	return lipgloss.Place(w, h, lipgloss.Center, lipgloss.Center, panel)
}

// refresh re-renders the transcript into the viewport. When follow mode is on
// (default) the view pins to the newest line; when the user has scrolled up,
// follow is off and the current offset is preserved so reading isn't yanked.
func (s *session) refresh() {
	s.viewport.SetContent(s.renderBlocks())
	if s.follow {
		s.viewport.GotoBottom()
	}
}

// ---------------------------------------------------------------------------
// log helpers: push a block, then refresh
// ---------------------------------------------------------------------------

func (s *session) logUser(text string) {
	b := s.push(blkUser)
	b.text.WriteString(text)
	s.refresh()
}

// logInfo / logSuccess / logWarn go to the ephemeral toast rail so operational
// chatter does not pollute the transcript or kill the welcome screen.
func (s *session) logInfo(text string) {
	s.setToast(toastInfo, text)
}

func (s *session) logSuccess(text string) {
	s.setToast(toastSuccess, text)
}

func (s *session) logWarn(text string) {
	s.setToast(toastWarn, text)
}

// logError stays in the transcript — real failures should remain reviewable —
// and also flashes a toast so the user notices without scrolling.
func (s *session) logError(text string) {
	b := s.push(blkError)
	b.text.WriteString(text)
	s.refresh()
	s.setToast(toastError, text)
}

// logPersist writes a lasting info/success line into the transcript (stats,
// multi-line reports). Prefer this over logInfo when the user needs to scroll
// back to the content.
func (s *session) logPersist(kind blockKind, text string) {
	b := s.push(kind)
	b.text.WriteString(text)
	s.refresh()
}

func (s *session) logTool(name, args string, sub bool) *block {
	b := s.push(blkTool)
	b.name = name
	b.args = args
	b.sub = sub
	b.started = time.Now()
	s.refresh()
	return b
}

// captureTodos parses a todo_write args blob and stores the latest todo list in
// s.todos so the pinned panel always shows current state. The agent rewrites
// the full list on every todo_write, so the latest call wins.
func (s *session) captureTodos(args string) {
	todos := argObjArrField(args, "todos")
	if todos == nil {
		return
	}
	// Stash a copy so later block mutations can't alias it.
	cp := make([]map[string]json.RawMessage, len(todos))
	copy(cp, todos)
	s.todos = cp
}

// allTodosComplete reports whether every captured todo is marked completed.
// Used to auto-dismiss the pinned tasks panel once the whole plan is done — a
// finished plan shouldn't linger as a permanent fixture. A later todo_write
// (new work) re-shows it.
func (s *session) allTodosComplete() bool {
	if len(s.todos) == 0 {
		return false
	}
	done, pend, run := countTodoStatuses(s.todos)
	return pend == 0 && run == 0 && done == len(s.todos)
}

// maxStoredOutput bounds the tool-result text retained in a block. A multi-MB
// result (e.g. a huge file dump) is stored verbatim though only ~3 lines ever
// render; this caps retention so one result can't pin megabytes of memory for
// the session. The renderer already truncates the visible portion.
const maxStoredOutput = 256 * 1024 // 256 KiB

// capOutput truncates a stored tool-result string to maxStoredOutput bytes,
// appending a marker when it cut content.
func capOutput(s string) string {
	if len(s) <= maxStoredOutput {
		return s
	}
	return s[:maxStoredOutput] + "\n…[truncated]"
}

func (s *session) logToolResult(output string) {
	b := s.push(blkToolResult)
	b.output = capOutput(output)
	s.refresh()
}

func (s *session) logApproveDiff(tool, args, diff string) {
	b := s.push(blkApprove)
	b.name, b.args, b.diff = tool, args, diff
	s.refresh()
}

// resolveLatestApproval makes the transcript truthful after the sticky
// approval prompt is dismissed. Call it with the core decision ("yes",
// "always", or "no") or its equivalent UI label before clearing the prompt.
func (s *session) resolveLatestApproval(decision string) {
	for i := len(s.blocks) - 1; i >= 0; i-- {
		b := s.blocks[i]
		if b == nil || b.kind != blkApprove || b.approval != approvalPending {
			continue
		}
		switch decision {
		case "yes", "approved once":
			b.approval = approvalApproved
		case "always", "always allowed":
			b.approval = approvalAlways
		case "no", "denied":
			b.approval = approvalDenied
		default:
			return
		}
		s.invalidateAll()
		s.refresh()
		return
	}
}

// logRaw pushes a pre-styled string verbatim (no further wrapping/styling).
func (s *session) logRaw(styled string) {
	b := s.push(blkRaw)
	b.text.WriteString(styled)
	s.refresh()
}

// ---------------------------------------------------------------------------
// Block rendering — flat, modern, gutter-free
//
// Conversation turns render as a coloured role glyph + bold role label on its
// own line, then full-width markdown content. Transient status lines
// (info/success/warn/error) collapse to a single glyph-prefixed line — no
// boxed card around every message. Tool output gets a left `│` rule panel.
// ---------------------------------------------------------------------------

// renderBlock renders a block, throttling re-renders of the LIVE streaming
// block (s.cur) so a long reply isn't O(L^2): reuse the last render until the
// text grows by >= streamBatch bytes or a newline / width change forces a fresh
// full render. Every actual render is a complete re-render of the current text,
// so this is purely a frequency cap (<= streamBatch bytes of display latency),
// not a correctness compromise. Finalized blocks render fully once and are
// cached upstream in renderBlocks.
func (s *session) renderBlock(b *block, w int) string {
	if b == s.cur && (b.kind == blkAssistant || b.kind == blkThinking) {
		text := b.text.String()
		force := w != b.renderW || strings.HasSuffix(text, "\n") || b.renderStr == ""
		if !force && len(text)-b.renderLen < streamBatch {
			return b.renderStr
		}
		out := s.decorateFocusedBlock(b, s.renderKeyHints(s.renderBlockFull(b, w)))
		b.renderW = w
		b.renderStr = out
		b.renderLen = len(text)
		return out
	}
	return s.decorateFocusedBlock(b, s.renderKeyHints(s.renderBlockFull(b, w)))
}

func (s *session) decorateFocusedBlock(b *block, out string) string {
	if s.focusedBlock >= 0 && s.focusedBlock < len(s.blocks) && s.blocks[s.focusedBlock] == b {
		return accentStyle.Render("▸ focused") + "\n" + out
	}
	return out
}

func (s *session) renderKeyHints(out string) string {
	if key := s.keyLabel("toggle_tool_output"); key != "" {
		out = strings.ReplaceAll(out, "ctrl+o", key)
	}
	if key := s.keyLabel("toggle_reasoning"); key != "" {
		out = strings.ReplaceAll(out, "ctrl+t", key)
	}
	return out
}

const streamBatch = 64 // P1-12: bytes of streaming growth between full re-renders

func (s *session) renderBlockFull(b *block, w int) string {
	switch b.kind {
	case blkUser:
		return roleLine("●", "you", "", c.user) + "\n" + renderMarkdown(b.text.String(), w)
	case blkAssistant:
		meta := b.model
		if meta == "" && len(s.models) > 0 && s.modelIdx >= 0 && s.modelIdx < len(s.models) {
			meta = s.models[s.modelIdx].ID
		}
		return roleLine("◆", "leader", meta, c.accent) + "\n" + renderMarkdown(b.text.String(), w)
	case blkThinking:
		if b.collapsed {
			n := strings.Count(b.text.String(), "\n") + 1
			if b.text.Len() == 0 {
				n = 0
			}
			return dimStyle.Render(fmt.Sprintf("▷ reasoning · %d line%s  (ctrl+t expand)", n, pluralS(n)))
		}
		return roleLine("◇", "reasoning", "", c.dim) + "\n" +
			thinkStyle.Render(renderMarkdown(b.text.String(), w))
	case blkTool:
		return s.renderToolBlock(b, w)
	case blkToolResult:
		return roleLine("▹", "result", "", c.success) + "\n" +
			renderOutputPanel(strings.TrimSpace(b.output), b.expanded, w, "lines", false)
	case blkSuccess:
		return renderStatusLine("✓ ", successStyle, baseStyle, b.text.String(), w)
	case blkWarn:
		return renderStatusLine("! ", warnStyle, baseStyle, b.text.String(), w)
	case blkError:
		// Multi-line errors (core crashes, stack traces) get a red rule panel so
		// real failures stand out from the success/info toasts around them.
		text := strings.TrimSpace(b.text.String())
		if strings.Contains(text, "\n") {
			parts := strings.SplitN(text, "\n", 2)
			head := renderStatusLine("✗ ", errStyle, baseStyle, parts[0], w)
			if len(parts) > 1 {
				head += "\n" + panelLines(parts[1], w, errRuleStyle, errOutStyle)
			}
			return head
		}
		return renderStatusLine("✗ ", errStyle, baseStyle, text, w)
	case blkInfo:
		return renderStatusLine("· ", mutedStyle, mutedStyle, b.text.String(), w)
	case blkApprove:
		// compact history marker; the live decision is the sticky banner. Reuses
		// the per-tool head grammar so the marker names the target, not a JSON blob.
		prefix, style := "? awaiting approval ", warnStyle
		switch b.approval {
		case approvalApproved:
			prefix, style = "✓ approved ", successStyle
		case approvalAlways:
			prefix, style = "✓ approved always ", successStyle
		case approvalDenied:
			prefix, style = "✗ denied ", errStyle
		}
		head := style.Render(prefix) +
			toolNameStyleFor(b.name).Render(toolIcon(b.name)+" "+toolDisplayName(b.name))
		if ka := keyArgFor(b.name, b.args); ka != "" {
			head += toolDetailStyle.Render("  " + truncate(ka, 60))
		}
		if b.approval == approvalPending {
			var controls []string
			if key := s.keyHint("approve"); key != "" {
				controls = append(controls, "["+key+"] approve")
			}
			if key := s.keyHint("deny"); key != "" {
				controls = append(controls, "["+key+"] deny")
			}
			if key := s.keyHint("approve_always"); key != "" {
				controls = append(controls, "["+key+"] always")
			}
			if len(controls) > 0 {
				head += dimStyle.Render("   " + strings.Join(controls, "  "))
			}
		}
		if strings.TrimSpace(b.diff) != "" {
			head += "\n" + renderDiffPanel(b.diff, b.expanded, s.width, s.keyHint("toggle_tool_output"))
		}
		return head
	case blkTrimmed:
		return dimStyle.Italic(true).Render(fmt.Sprintf("⋯ %d older transcript blocks hidden · reopen the session to view full history", b.trimmed))
	case blkRaw:
		return b.text.String()
	}
	return ""
}

// renderStatusLine renders a toast-style status line (info/success/warn/error)
// with a 2-cell prefix, word-wrapping the body to the terminal width so long
// plugin descriptions / error messages aren't clipped mid-sentence.
func renderStatusLine(prefix string, prefixStyle, bodyStyle lipgloss.Style, text string, w int) string {
	const prefixCells = 2 // "✓ ", "· ", "! ", "✗ "
	avail := w - prefixCells
	if avail < 8 {
		avail = 8
	}
	text = strings.TrimRight(text, "\n")
	if text == "" {
		return prefixStyle.Render(prefix)
	}
	lines := strings.Split(wrapPlain(text, avail), "\n")
	var b strings.Builder
	pad := strings.Repeat(" ", prefixCells)
	for i, ln := range lines {
		if i == 0 {
			b.WriteString(prefixStyle.Render(prefix))
		} else {
			b.WriteString(pad)
		}
		b.WriteString(bodyStyle.Render(ln))
		if i < len(lines)-1 {
			b.WriteByte('\n')
		}
	}
	return b.String()
}

// renderToolBlock dispatches to a per-tool renderer so each call gets a layout
// that surfaces its most relevant info (the bash command, the file path, the
// grep pattern…) instead of a generic name(args) head. Sub-agent internal
// calls (spawn:*) collapse to a dim one-liner so a scout's chatter stays tidy.
// The shared header grammar (icon · name · keyarg … dur status) and the
// status badge live in tool_blocks.go; this method just picks the renderer.
func (s *session) renderToolBlock(b *block, w int) string {
	if b.sub {
		return renderSubToolLine(b, w)
	}
	switch b.name {
	case "bash":
		return renderBashBlock(b, w)
	case "read_file":
		return renderReadFileBlock(b, w)
	case "write_file":
		return renderWriteFileBlock(b, w)
	case "edit":
		return renderEditBlock(b, w)
	case "patch":
		return renderPatchBlock(b, w)
	case "list_dir":
		return renderListDirBlock(b, w)
	case "grep":
		return renderGrepBlock(b, w)
	case "glob":
		return renderGlobBlock(b, w)
	case "git_status":
		return renderGitStatusBlock(b, w)
	case "git_log":
		return renderGitLogBlock(b, w)
	case "git_diff":
		return renderGitDiffBlock(b, w)
	case "git_add":
		return renderGitAddBlock(b, w)
	case "git_commit":
		return renderGitCommitBlock(b, w)
	case "todo_write":
		return renderTodoWriteBlock(b, w)
	case "todo_read":
		return renderTodoReadBlock(b, w)
	case "diagnostics":
		return renderDiagnosticsBlock(b, w)
	case "fetch":
		return renderFetchBlock(b, w)
	case "memory":
		return renderMemoryBlock(b, w)
	case "spawn", "subagent":
		return renderSubagentBlock(b, w)
	case "bulk":
		return renderBulkBlock(b, w)
	default:
		return renderGenericToolBlock(b, w)
	}
}

// renderOutputPanel wraps tool output, truncating to the first 3 lines
// unless the block is expanded (ctrl+o). A dim hint line shows the toggle.
// `unit` labels the hidden remainder ("lines"/"matches"/"entries"/…) and `err`
// tints the panel red for failed calls. ctrl+o is per-block: users expand one
// call, not all. Upgrade path: a viewport pager if outputs grow huge.
func renderOutputPanel(output string, expanded bool, w int, unit string, err bool) string {
	if unit == "" {
		unit = "lines"
	}
	const headLines = 3
	lines := strings.Split(output, "\n")
	if len(lines) > headLines && !expanded {
		more := len(lines) - headLines
		shown := strings.Join(lines[:headLines], "\n")
		hint := dimStyle.Italic(true).Render(
			fmt.Sprintf("│ … +%d %s  (ctrl+o expand)", more, unit))
		return resultPanel(shown, w, err) + "\n" + hint
	}
	panel := resultPanel(output, w, err)
	if len(lines) > headLines && expanded {
		panel += "\n" + dimStyle.Italic(true).Render("│ (ctrl+o collapse)")
	}
	return panel
}

// resultPanel renders tool/command output with a left `│` rule, wrapped to fit.
// The rule + content are tinted red when err is set so failures are scannable.
func resultPanel(output string, w int, err bool) string {
	if err {
		return panelLines(output, w, errRuleStyle, errOutStyle)
	}
	return panelLines(output, w, dimStyle, resultStyle)
}

// lastToolOutputBlock returns the most recent block carrying tool output
// (a top-level blkTool with a result, or a standalone blkToolResult). Used
// by ctrl+o when pinned to the bottom.
func (s *session) lastToolOutputBlock() *block {
	for i := len(s.blocks) - 1; i >= 0; i-- {
		b := s.blocks[i]
		if b == nil {
			continue
		}
		if b.kind == blkToolResult {
			return b
		}
		if b.kind == blkTool && !b.sub && b.dur > 0 && strings.TrimSpace(b.output) != "" {
			return b
		}
	}
	return nil
}

// nearestToolOutputBlock picks the tool output nearest the visible viewport.
// renderBlocks records exact line ranges, avoiding the old scroll-percentage
// approximation (which selected the wrong card whenever block heights varied).
func (s *session) nearestToolOutputBlock() *block {
	var candidates []*block
	for _, b := range s.blocks {
		if b == nil {
			continue
		}
		if b.kind == blkToolResult {
			candidates = append(candidates, b)
			continue
		}
		if b.kind == blkApprove && strings.TrimSpace(b.diff) != "" {
			candidates = append(candidates, b)
			continue
		}
		if b.kind == blkTool && !b.sub && b.dur > 0 && strings.TrimSpace(b.output) != "" {
			candidates = append(candidates, b)
		}
	}
	if len(candidates) == 0 {
		return nil
	}
	if s.follow || s.viewport.AtBottom() {
		return candidates[len(candidates)-1]
	}
	top := s.viewport.YOffset()
	bottom := top + max(1, s.viewport.VisibleLineCount()) - 1
	center := (top + bottom) / 2
	best, bestDistance := candidates[0], int(^uint(0)>>1)
	for _, candidate := range candidates {
		distance := 0
		switch {
		case candidate.renderEnd < top:
			distance = top - candidate.renderEnd
		case candidate.renderStart > bottom:
			distance = candidate.renderStart - bottom
		default:
			blockCenter := (candidate.renderStart + candidate.renderEnd) / 2
			distance = blockCenter - center
			if distance < 0 {
				distance = -distance
			}
		}
		if distance < bestDistance {
			best, bestDistance = candidate, distance
		}
	}
	return best
}

// findSubProgress returns the live progress entry for runID, or nil.
func (s *session) findSubProgress(runID string) *subProgressEntry {
	for _, e := range s.subProgress {
		if e.runID == runID {
			return e
		}
	}
	return nil
}

// removeSubProgress drops the progress entry for runID (subagent finished).
func (s *session) removeSubProgress(runID string) {
	for i, e := range s.subProgress {
		if e.runID == runID {
			s.subProgress = append(s.subProgress[:i], s.subProgress[i+1:]...)
			return
		}
	}
}

// ---- active-task (scout) helpers ----

// isInFlight reports a top-level tool block still awaiting its result.
func isInFlight(b *block) bool {
	return b != nil && b.kind == blkTool && !b.sub && b.dur == 0
}

// hasLiveContent reports whether the transcript has content that changes over
// time (a streaming block or an in-flight tool). Used to gate the per-tick
// refresh so an idle session doesn't re-render the viewport every second.
func (s *session) hasLiveContent() bool {
	if s.cur != nil {
		return true
	}
	for _, b := range s.blocks {
		if b != nil && b.kind == blkTool && b.inFlight() {
			return true
		}
	}
	return false
}

// finalizeInFlight marks any still-running tool blocks as done so the
// active-tasks panel doesn't linger after an abort or turn end.
func (s *session) finalizeInFlight(note string) {
	changed := false
	for _, b := range s.blocks {
		if isInFlight(b) || (b != nil && b.kind == blkTool && b.sub && b.dur == 0) {
			b.dur = time.Since(b.started)
			if strings.TrimSpace(b.output) == "" {
				b.output = note
			}
			changed = true
		}
	}
	if changed {
		s.invalidateAll()
	}
}

// taskLabel extracts a human-readable description for an in-flight tool: the
// scout's prompt for a spawn, else the tool name + args.
func taskLabel(b *block, w int) string {
	if b.name == "spawn" {
		if p := jsonStringField(b.args, "prompt"); p != "" {
			return truncate(p, w-6)
		}
	}
	return b.name + "(" + truncate(b.args, w-len(b.name)-8) + ")"
}

// taskRole labels the agent kind for the active-tasks panel: a spawn is a scout.
func taskRole(b *block) string {
	if b.name == "spawn" {
		return "scout"
	}
	return b.name
}

// taskModel returns the model a scout runs on (parsed from spawn args), if any.
func taskModel(b *block) string {
	if b.name == "spawn" {
		return jsonStringField(b.args, "model")
	}
	return ""
}

// formatDur renders a duration as M:SS.
func formatDur(d time.Duration) string {
	m := int(d.Minutes())
	s := int(d.Seconds()) % 60
	return fmt.Sprintf("%d:%02d", m, s)
}

// jsonStringField reads a string field from a JSON object string.
func jsonStringField(s, key string) string {
	var m map[string]json.RawMessage
	if json.Unmarshal([]byte(s), &m) != nil {
		return ""
	}
	return get(m, key)
}

// roleLine renders a coloured glyph + bold role label + dim metadata.
//
//	● you   · 14:32
func roleLine(glyph, role, meta, color string) string {
	out := lipgloss.NewStyle().Foreground(lipgloss.Color(color)).Bold(true).Render(glyph + " " + role)
	if meta != "" {
		out += mutedStyle.Render("  " + truncate(meta, 40))
	}
	return out
}

// ---------------------------------------------------------------------------
// Welcome screen — shown when the conversation is empty.
//
// A centred brand + tagline + a selectable list of starter prompts. Arrow keys
// (or number keys) pick one; enter fills the input so it can be edited before
// sending.
// ---------------------------------------------------------------------------

var welcomeExamples = []string{
	"Explain how this codebase is organized",
	"Write a unit test for a core module",
	"Review my code for potential issues",
	"Refactor the most complex function for readability",
}

func (s *session) renderWelcome() string {
	w := s.viewport.Width()
	h := s.viewport.Height()

	brand := accentStyle.Render("◆ ") + boldBaseStyle.Render("Catalyst") + dimStyle.Render(" Code")
	sub := mutedStyle.Render("a multi-provider coding agent")
	if s.coreLifecycle == coreStarting && s.coreStartGen > 0 {
		panelW := min(50, max(20, w-4))
		panel := lipgloss.NewStyle().
			BorderStyle(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color(c.dim)).
			Padding(0, 1).
			Width(panelW).
			Render(accentStyle.Render("◷ Starting…") + "\n\n" +
				baseStyle.Render("Connecting to the core and checking credentials."))
		return lipgloss.Place(w, h, lipgloss.Center, lipgloss.Center, brand+"\n"+sub+"\n\n"+panel)
	}

	// Unauthed first-run: lead with login instead of example prompts.
	if !s.authed {
		panelW := 50
		if w-4 < panelW {
			panelW = w - 4
		}
		if panelW < 30 {
			panelW = 30
		}
		rows := []string{
			accentStyle.Render("◆ Get started"),
			"",
			baseStyle.Render("No API key yet — log in to start chatting."),
			"",
			dimStyle.Render("Enter opens /login · / for commands · ? help"),
		}
		panel := lipgloss.NewStyle().
			BorderStyle(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color(c.dim)).
			Padding(0, 1).
			Width(panelW).
			Render(strings.Join(rows, "\n"))
		content := brand + "\n" + sub + "\n\n" + panel
		return lipgloss.Place(w, h, lipgloss.Center, lipgloss.Center, content)
	}

	// build the example panel
	panelW := 50
	if w-4 < panelW {
		panelW = w - 4
	}
	if panelW < 30 {
		panelW = 30
	}
	var rows []string
	rows = append(rows, accentStyle.Render("◆ Examples"))
	for i, ex := range welcomeExamples {
		marker := "  "
		if i == s.welcomeIdx {
			marker = accentStyle.Render("▸ ")
		}
		num := dimStyle.Render(fmt.Sprintf("%d.", i+1))
		text := baseStyle.Render(ex)
		if i == s.welcomeIdx {
			text = accentStyle.Render(ex)
		}
		row := marker + num + " " + text
		if i == s.welcomeIdx {
			row = lipgloss.NewStyle().Background(lipgloss.Color(c.bg)).Width(panelW).Render(row)
		}
		rows = append(rows, row)
	}
	rows = append(rows, "")
	rows = append(rows, dimStyle.Render("↑↓ pick · enter to use · / commands · ? help"))
	panel := strings.Join(rows, "\n")

	// wrap the panel in a subtle rounded border
	panel = lipgloss.NewStyle().
		BorderStyle(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(c.dim)).
		Padding(0, 1).
		Width(panelW).
		Render(panel)

	content := brand + "\n" + sub + "\n\n" + panel
	return lipgloss.Place(w, h, lipgloss.Center, lipgloss.Center, content)
}

// rebuildBlocksFromHistory reconstructs the transcript blocks from a loaded
// session's message list (role / content / reasoning_content / tool_calls /
// tool results) so a resumed or switched session shows its prior turns instead
// of an empty view. Timing for historical tool calls is unknown, so they render
// as finalized cards without a duration line (gated on started.IsZero()).
func (s *session) rebuildBlocksFromHistory(msgs []map[string]json.RawMessage) {
	s.blocks = nil
	s.cur = nil
	s.cacheIdx = 0
	pending := map[string]*block{} // tool_call_id -> block awaiting its result
	for _, msg := range msgs {
		switch get(msg, "role") {
		case "user":
			text := contentText(msg["content"])
			if strings.TrimSpace(text) == "" {
				continue
			}
			b := s.push(blkUser)
			b.text.WriteString(text)
		case "assistant":
			if r := contentText(msg["reasoning_content"]); strings.TrimSpace(r) != "" {
				b := s.push(blkThinking)
				b.text.WriteString(r)
			}
			if c := contentText(msg["content"]); strings.TrimSpace(c) != "" {
				b := s.push(blkAssistant)
				b.text.WriteString(c)
				if s.modelIdx >= 0 && s.modelIdx < len(s.models) {
					b.model = s.models[s.modelIdx].ID
				}
			}
			if raw, ok := msg["tool_calls"]; ok {
				var calls []map[string]json.RawMessage
				if json.Unmarshal(raw, &calls) == nil {
					for _, tc := range calls {
						name, args, id := decodeToolCall(tc)
						if name == "" {
							continue
						}
						sub := strings.HasPrefix(name, "spawn:")
						disp := name
						if sub {
							disp = strings.TrimPrefix(name, "spawn:")
						}
						b := s.push(blkTool)
						b.name = disp
						b.args = args
						b.sub = sub
						b.id = id
						b.started = time.Time{} // historical: no timing
						b.dur = 1               // >0 => finalized, not in-flight
						if disp == "todo_write" {
							s.captureTodos(args) // last todo_write wins
						}
						if id != "" {
							pending[id] = b
						}
					}
				}
			}
		case "tool":
			out := contentText(msg["content"])
			id := get(msg, "tool_call_id")
			if b, ok := pending[id]; ok && id != "" {
				b.output = capOutput(out)
				delete(pending, id)
			} else {
				b := s.push(blkToolResult)
				b.output = capOutput(out)
			}
		}
	}
	s.cur = nil
}

// contentText extracts displayable text from a message content field, which may
// be a plain string, null, or a multimodal array of text/image parts.
func contentText(raw json.RawMessage) string {
	if len(raw) == 0 || string(raw) == "null" {
		return ""
	}
	var s string
	if json.Unmarshal(raw, &s) == nil {
		return s
	}
	var parts []map[string]json.RawMessage
	if json.Unmarshal(raw, &parts) == nil {
		var b strings.Builder
		imgs := 0
		for _, p := range parts {
			switch get(p, "type") {
			case "text":
				if b.Len() > 0 {
					b.WriteByte('\n')
				}
				b.WriteString(get(p, "text"))
			case "image_url":
				imgs++
			}
		}
		if imgs > 0 {
			if b.Len() > 0 {
				b.WriteByte('\n')
			}
			fmt.Fprintf(&b, "[image \u00d7%d]", imgs)
		}
		return b.String()
	}
	return ""
}

// decodeToolCall pulls (name, arguments, id) from a tool_calls entry.
func decodeToolCall(tc map[string]json.RawMessage) (name, args, id string) {
	id = get(tc, "id")
	var fn map[string]json.RawMessage
	if raw, ok := tc["function"]; ok {
		if json.Unmarshal(raw, &fn) == nil {
			name = get(fn, "name")
			args = get(fn, "arguments")
		}
	}
	return name, args, id
}
