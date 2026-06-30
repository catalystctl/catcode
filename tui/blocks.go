package main

import (
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/charmbracelet/lipgloss"
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
	blkRaw // pre-styled string rendered verbatim (e.g. the /model list)
)

type block struct {
	kind      blockKind
	text      strings.Builder // streamed content (assistant / thinking) or raw
	name      string          // tool name (blkTool / blkApprove)
	args      string          // tool args
	output    string          // blkToolResult
	diff      string          // blkTool: unified-diff text (tool_result only); shown instead of output when present
	started   time.Time       // blkTool: when the call began
	dur       time.Duration   // blkTool: elapsed until result
	collapsed bool            // blkThinking only
	model     string          // blkAssistant: model id (for the role line)
	sub       bool            // blkTool: a spawn:* sub-agent internal call (collapsed)
	id        string          // blkTool: tool-call id, to match results out of order
	expanded  bool            // blkTool / blkToolResult: full output shown (ctrl+o)
	renderW   int             // P1-12: width the streaming block was last rendered at
	renderLen int             // P1-12: text length at last render (throttle)
	renderStr string          // P1-12: cached render of the streaming block
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
		trim := len(s.blocks) - maxBlocks
		s.blocks = s.blocks[trim:]
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
}

// renderBlocks returns the full viewport content. Finalized blocks are cached
// in s.cache and only extended; the live streaming block (s.cur) and any
// in-flight tool blocks (awaiting their result) are re-rendered each call so
// the active-tasks panel's elapsed time stays live.
// ponytail: full re-render (invalidateAll) only on resize/toggle; very long
// histories re-wrap the current streaming block per token (upgrade: cache
// wrapped lines per finalized block keyed by width).
func (s *session) renderBlocks() string {
	if len(s.blocks) == 0 {
		return s.renderWelcome()
	}
	w := s.viewport.Width
	for s.cacheIdx < len(s.blocks) {
		blk := s.blocks[s.cacheIdx]
		if blk == s.cur || isInFlight(blk) {
			break // don't cache the live streaming block or in-flight tools
		}
		s.cache.WriteString(s.renderBlock(blk, w))
		s.cache.WriteString("\n\n") // breathing room between blocks
		s.cacheIdx++
	}
	var b strings.Builder
	b.WriteString(s.cache.String())
	if s.cur != nil {
		b.WriteString(s.renderBlock(s.cur, w))
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
		b.WriteString(s.renderBlock(blk, w))
		b.WriteString("\n\n")
	}
	return strings.TrimRight(b.String(), "\n")
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

func (s *session) logInfo(text string) {
	b := s.push(blkInfo)
	b.text.WriteString(text)
	s.refresh()
}

func (s *session) logSuccess(text string) {
	b := s.push(blkSuccess)
	b.text.WriteString(text)
	s.refresh()
}

func (s *session) logWarn(text string) {
	b := s.push(blkWarn)
	b.text.WriteString(text)
	s.refresh()
}

func (s *session) logError(text string) {
	b := s.push(blkError)
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

func (s *session) logToolResult(output string) {
	b := s.push(blkToolResult)
	b.output = output
	s.refresh()
}

func (s *session) logApproveDiff(tool, args, diff string) {
	b := s.push(blkApprove)
	b.name, b.args, b.diff = tool, args, diff
	s.refresh()
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
		out := s.renderBlockFull(b, w)
		b.renderW = w
		b.renderStr = out
		b.renderLen = len(text)
		return out
	}
	return s.renderBlockFull(b, w)
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
			return dimStyle.Render(fmt.Sprintf("▷ reasoning · %d line(s)  (ctrl+t expand)", n))
		}
		return roleLine("◇", "reasoning", "", c.dim) + "\n" +
			thinkStyle.Render(renderMarkdown(b.text.String(), w))
	case blkTool:
		return s.renderToolBlock(b, w)
	case blkToolResult:
		return roleLine("▹", "result", "", c.success) + "\n" +
			renderOutputPanel(strings.TrimSpace(b.output), b.expanded, w)
	case blkSuccess:
		return successStyle.Render("✓ ") + baseStyle.Render(b.text.String())
	case blkWarn:
		return warnStyle.Render("! ") + baseStyle.Render(b.text.String())
	case blkError:
		return errStyle.Render("✗ ") + baseStyle.Render(b.text.String())
	case blkInfo:
		return mutedStyle.Render("· ") + mutedStyle.Render(b.text.String())
	case blkApprove:
		// compact history marker; the live decision is the sticky banner
		head := warnStyle.Render("? approve ") + toolNameStyle.Render(b.name) +
			toolDetailStyle.Render("("+truncate(b.args, 60)+")")
		head += dimStyle.Render("   [y]es  [n]o  [a]lways")
		if strings.TrimSpace(b.diff) != "" {
			head += "\n" + renderDiffPanel(b.diff, b.expanded, s.width)
		}
		return head
	case blkRaw:
		return b.text.String()
	}
	return ""
}

// renderToolBlock: role line with name(args) · duration, then an output panel.
// Sub-agent internal calls (spawn:*) collapse to a dim one-liner so a scout's
// internal tool chatter doesn't drown the transcript. In-flight tools (no
// result yet) render just the head; the active-tasks panel shows live elapsed
// time for scouts.
func (s *session) renderToolBlock(b *block, w int) string {
	if b.sub {
		head := dimStyle.Render("  ┊ " + b.name)
		if b.args != "" {
			head += dimStyle.Render("(" + truncate(b.args, w-10) + ")")
		}
		if b.dur > 0 && !b.started.IsZero() {
			head += dimStyle.Render(fmt.Sprintf(" · %.1fs", b.dur.Seconds()))
		}
		return head
	}
	var head strings.Builder
	head.WriteString(toolNameStyle.Render("▸ " + b.name))
	if b.args != "" {
		head.WriteString(toolDetailStyle.Render("(" + truncate(b.args, w-12) + ")"))
	}
	if b.dur > 0 && !b.started.IsZero() {
		head.WriteString(dimStyle.Render(fmt.Sprintf(" · %.1fs", b.dur.Seconds())))
	}
	if b.dur == 0 {
		return head.String() // in-flight: head only; live status is in the panel
	}
	// Prefer a unified-diff view when the core attached one (edit/patch/
	// write_file); fall back to the plain output panel, then to "(no output)" only
	// when both are empty. ctrl+o toggles b.expanded for either view.
	body := ""
	switch {
	case strings.TrimSpace(b.diff) != "":
		body = "\n" + renderDiffPanel(b.diff, b.expanded, w)
	case strings.TrimSpace(b.output) != "":
		body = "\n" + renderOutputPanel(strings.TrimSpace(b.output), b.expanded, w)
	default:
		body = "\n" + dimStyle.Italic(true).Render("│ (no output)")
	}
	return head.String() + body
}

// renderOutputPanel wraps tool output, truncating to the first 3 lines
// unless the block is expanded (ctrl+o). A dim hint line shows the toggle.
// ponytail: 3-line truncation is the laziest answer to "too much output"; a
// per-block expand key beats a global fold because users expand one call, not
// all. Upgrade path: a viewport pager if outputs grow huge.
func renderOutputPanel(output string, expanded bool, w int) string {
	const headLines = 3
	lines := strings.Split(output, "\n")
	if len(lines) > headLines && !expanded {
		more := len(lines) - headLines
		shown := strings.Join(lines[:headLines], "\n")
		hint := dimStyle.Italic(true).Render(
			fmt.Sprintf("│ … +%d line(s)  (ctrl+o expand)", more))
		return resultPanel(shown, w) + "\n" + hint
	}
	panel := resultPanel(output, w)
	if len(lines) > headLines && expanded {
		panel += "\n" + dimStyle.Italic(true).Render("│ (ctrl+o collapse)")
	}
	return panel
}

// resultPanel renders tool/command output with a dim left rule, wrapped to fit.
func resultPanel(output string, w int) string {
	contentW := w - 3 // "│ " prefix + content
	if contentW < 2 {
		contentW = 2
	}
	rule := dimStyle.Render("│ ")
	wrapped := wrapPlain(output, contentW)
	var b strings.Builder
	for _, l := range strings.Split(wrapped, "\n") {
		b.WriteString(rule)
		b.WriteString(resultStyle.Render(l))
		b.WriteByte('\n')
	}
	return strings.TrimRight(b.String(), "\n")
}

// lastToolOutputBlock returns the most recent block carrying tool output
// (a top-level blkTool with a result, or a standalone blkToolResult). Used
// by ctrl+o to expand the call the user just saw.
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
	w := s.viewport.Width
	h := s.viewport.Height

	brand := accentStyle.Render("◆ ") + boldBaseStyle.Render("Umans") + dimStyle.Render(" harness")
	sub := mutedStyle.Render("an OpenAI-compatible coding agent")

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
				b.output = out
				delete(pending, id)
			} else {
				b := s.push(blkToolResult)
				b.output = out
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
