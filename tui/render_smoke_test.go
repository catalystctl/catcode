package main

import (
	"fmt"
	"strings"
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"
)

func TestRenderSmoke(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width = 90
	s.height = 30
	s.authed = true
	s.approvalModeStr = "destructive"
	s.turnCount = 3
	s.models = []modelInfo{
		{ID: "umans-glm-5.2", ContextWindow: 131072, MaxTokens: 8192},
		{ID: "umans-glm-5.1", ContextWindow: 65536, MaxTokens: 4096},
	}
	s.modelIdx = 0
	s.layout()

	// welcome screen: action-oriented selectable examples; branding stays in the header
	welcome := stripANSI(s.renderBlocks())
	for _, want := range []string{"What would you like to build?", "Understand", "Review recent"} {
		if !strings.Contains(welcome, want) {
			t.Errorf("welcome missing %q:\n%s", want, welcome)
		}
	}
	t.Logf("WELCOME:\n%s", welcome)

	s.logUser("Explain the diff between channels and mutexes in Go, with a short example each.")
	s.logInfo("2 model(s) discovered")
	b := s.push(blkAssistant)
	b.model = "umans-glm-5.2"
	b.text.WriteString("Channels pass ownership; a mutex guards shared state. Use a channel when one goroutine produces values and another consumes them; use a mutex when multiple goroutines read/write the same variable.")
	s.cur = nil
	s.invalidateAll()

	tb := s.push(blkTool)
	tb.name = "bash"
	tb.args = "go test ./..."
	tb.output = "ok  catalyst-code-tui  0.012s\nPASS"
	tb.dur = 1400000000
	s.invalidateAll()

	s.push(blkError).text.WriteString("not authenticated — run /login first")
	s.invalidateAll()

	// Flat layout: tool calls form a compact activity run. The command stays
	// visible, while output waits behind the per-call details toggle.
	blocks := stripANSI(s.renderBlocks())
	for _, want := range []string{"Explain the diff", "glm-5.2", "activity", "1 call", "bash", "go test", "details", "✗", "not authenticated"} {
		if !strings.Contains(blocks, want) {
			t.Errorf("blocks missing %q:\n%s", want, blocks)
		}
	}
	if strings.Contains(blocks, "PASS") {
		t.Errorf("compact activity row leaked full tool output:\n%s", blocks)
	}
	tb.expanded = true
	s.invalidateAll()
	detailed := stripANSI(s.renderBlocks())
	if !strings.Contains(detailed, "PASS") || !strings.Contains(detailed, "$ go test ./...") {
		t.Errorf("expanded activity call should restore full output:\n%s", detailed)
	}
	t.Logf("BLOCKS:\n%s", blocks)

	// header: one-row brand + cwd + operational identity
	hdr := stripANSI(s.renderHeader())
	for _, want := range []string{"Catalyst", "ready", "umans-glm-5.2"} {
		if !strings.Contains(hdr, want) {
			t.Errorf("header missing %q:\n%s", want, hdr)
		}
	}
	t.Logf("HEADER:\n%s", hdr)

	// footer: contextual controls + context budget (toasts temporarily replace controls)
	s.toast = nil
	ftr := stripANSI(s.renderFooter())
	for _, want := range []string{"Enter", "send", "commands", "0%"} {
		if !strings.Contains(ftr, want) {
			t.Errorf("footer missing %q:\n%s", want, ftr)
		}
	}
	t.Logf("FOOTER:\n%s", ftr)

	// position rail is absent at the bottom; it only appears while reading history.
	pos := stripANSI(s.renderPositionBar())
	if pos != "" {
		t.Errorf("position bar should be hidden when pinned:\n%s", pos)
	}
	t.Logf("POSITION:\n%s", pos)

	s.pendingApproval = &approvalPrompt{requestID: "r1", tool: "bash", args: "rm -rf /tmp/cache"}
	banner := stripANSI(s.renderApprovalBanner())
	if !strings.Contains(banner, "approval required") || !strings.Contains(banner, "bash") {
		t.Errorf("approval banner missing content:\n%s", banner)
	}
	t.Logf("BANNER:\n%s", banner)

	// Keep the palette assertion independent of commands persisted by other
	// tests or a developer's local TUI session.
	s.recentCommands = []string{"/approval"}
	s.openCommandPalette()
	pal := stripANSI(s.renderModalBody())
	if !strings.Contains(pal, "Command Palette") || !strings.Contains(pal, "/approval") {
		t.Errorf("palette missing content:\n%s", pal)
	}
	t.Logf("PALETTE:\n%s", pal)

	s.openModelPicker()
	mp := stripANSI(s.renderModalBody())
	if !strings.Contains(mp, "Models") || !strings.Contains(mp, "umans-glm-5.2") {
		t.Errorf("model picker missing content:\n%s", mp)
	}
	t.Logf("MODELS:\n%s", mp)
}

func TestScrollFollow(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width = 80
	s.height = 12
	s.models = []modelInfo{{ID: "m1"}}
	s.modelIdx = 0
	s.layout()

	// tall content so the viewport scrolls
	s.logUser(strings.Repeat("line of text\n", 40))
	s.invalidateAll()
	s.refresh()

	// default: follow on, pinned to the bottom
	if !s.follow {
		t.Fatal("follow should be on by default")
	}
	if !s.viewport.AtBottom() {
		t.Fatal("viewport should be pinned to bottom initially")
	}

	// scroll up: follow pauses, position bar shows the new-lines affordance
	s.handleScrollKey(keyMsg("pgup"))
	if s.follow {
		t.Error("pgup should pause follow")
	}
	pos := stripANSI(s.renderPositionBar())
	if !strings.Contains(pos, "new") {
		t.Errorf("scrolled-up position bar should show new-lines hint:\n%s", pos)
	}
	t.Logf("SCROLLED POS:\n%s", pos)

	// jump to bottom: follow re-pins
	s.handleScrollKey(keyMsg("ctrl+end"))
	if !s.follow {
		t.Error("ctrl+end should re-pin follow")
	}
	if !s.viewport.AtBottom() {
		t.Error("ctrl+end should land at the bottom")
	}

	// streaming while paused keeps the view offset (no yank)
	s.follow = false
	yoff := s.viewport.YOffset()
	s.logInfo("a new line arrives while reading")
	if s.viewport.YOffset() != yoff {
		t.Errorf("view yanked while paused: was %d now %d", yoff, s.viewport.YOffset())
	}
}

func TestMarkdown(t *testing.T) {
	out := stripANSI(renderMarkdown("Here is `inline` code and **bold** and *italic*.\n\n```go\nch := make(chan int)\n```", 60))
	for _, want := range []string{"inline", "bold", "italic", "ch := make(chan int)"} {
		if !strings.Contains(out, want) {
			t.Errorf("markdown missing %q:\n%s", want, out)
		}
	}
	t.Logf("MARKDOWN:\n%s", out)

	// code block renders with chroma styling (no custom left-rule required)
	long := stripANSI(renderMarkdown("```sh\n"+strings.Repeat("x", 200)+"\n```", 40))
	if !strings.Contains(long, "xxx") {
		t.Errorf("code block missing content:\n%s", long)
	}
}

func TestThemeAndMetrics(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width = 70
	s.height = 24
	s.models = []modelInfo{{ID: "m1", ContextWindow: 8192}}
	s.modelIdx = 0
	s.layout()

	for _, name := range themeNames() {
		if !setTheme(name) {
			t.Fatalf("setTheme(%q) failed", name)
		}
		s.invalidateAll()
		s.refresh()
		out := stripANSI(s.renderHeader())
		if !strings.Contains(out, "Catalyst") {
			t.Errorf("theme %q header broke: %s", name, out)
		}
	}
	setTheme("mocha")

	// metrics: footer shows rounded throughput (tps) + ttft; cumulative token
	// totals live in /stats + the debug log. The context budget is separate.
	s.lastMetrics = []byte(`{"tps":"42.1","ttft_ms":"180","tokens_in":"1000","tokens_out":"200"}`)
	s.contextTokens = 1200
	m := s.renderMetrics()
	for _, w := range []string{"tok/s", "ttft"} {
		if !strings.Contains(m, w) {
			t.Errorf("metrics missing %q in %q", w, m)
		}
	}
	// tps is rounded to the nearest integer: 42.1 → "42 tok/s · 180ms ttft".
	if !strings.Contains(m, "42 tok/s") {
		t.Errorf("metrics tps should be rounded to int, got %q", m)
	}
	s.lastMetrics = []byte(`{"tps_est":"51.7","ttft_ms":"180","tokens_in":"1000","tokens_out":"200"}`)
	m = s.renderMetrics()
	if !strings.Contains(m, "~52 tok/s") {
		t.Errorf("live estimated tps should be rounded and marked approximate, got %q", m)
	}
	// context budget: live context / model window → "14% 1.2k/8.2k"
	ctx := s.renderContext()
	for _, w := range []string{"%", "1.2k", "8.2k"} {
		if !strings.Contains(ctx, w) {
			t.Errorf("context missing %q in %q", w, ctx)
		}
	}
	t.Logf("METRICS: %s  CONTEXT: %s", m, ctx)

	// Umans live concurrency is shown AHEAD of tps ("Conc 3/8 · 42 tok/s …")
	// and is rendered even when idle (no turn metrics), because it is polled
	// independently. A nil limit (unlimited plan) renders as ∞. It only shows
	// when the selected model routes to the Umans provider the poll tracks.
	var used int64 = 3
	var limit int64 = 8
	s.lastMetrics = nil // idle: no turn metrics yet
	s.umansConcUsed = &used
	s.umansConcLimit = &limit
	s.umansConcProvider = "umans"
	s.models = []modelInfo{{ID: "umans-glm-5.2", Provider: "umans"}}
	s.modelIdx = 0
	mc := s.renderMetrics()
	if !strings.Contains(mc, "Conc 3/8") {
		t.Errorf("conc should render when a umans model is selected, got %q", mc)
	}
	// conc must precede tps when both are present.
	s.lastMetrics = []byte(`{"tps":"42.1","ttft_ms":"180"}`)
	mc = s.renderMetrics()
	if ci, ti := strings.Index(mc, "Conc 3/8"), strings.Index(mc, "42 tok/s"); ci < 0 || ti < 0 || ci > ti {
		t.Errorf("conc should appear before tps, got %q", mc)
	}
	// unlimited plan → ∞.
	s.umansConcLimit = nil
	mc = s.renderMetrics()
	if !strings.Contains(mc, "Conc 3/∞") {
		t.Errorf("unlimited limit should render ∞, got %q", mc)
	}
	// a non-Umans model selected (e.g. Gemini) → hidden, even with a live value.
	s.models = []modelInfo{{ID: "gemini-2.5-pro", Provider: "gemini"}}
	s.umansConcLimit = &limit
	mc = s.renderMetrics()
	if strings.Contains(mc, "Conc") {
		t.Errorf("conc should be hidden when a non-umans model is selected, got %q", mc)
	}
	// not Umans / fetch failed (used nil) → hidden entirely.
	s.models = []modelInfo{{ID: "umans-glm-5.2", Provider: "umans"}}
	s.umansConcUsed = nil
	mc = s.renderMetrics()
	if strings.Contains(mc, "Conc") {
		t.Errorf("conc should be hidden when used is nil, got %q", mc)
	}

	// long model list scrolls + highlights
	s.width = 60
	s.height = 16
	s.models = make([]modelInfo, 30)
	for i := range s.models {
		s.models[i] = modelInfo{ID: fmt.Sprintf("model-%d", i), ContextWindow: 8192, MaxTokens: 2048}
	}
	s.openModelPicker()
	s.modal.cursor = 12
	body := stripANSI(s.renderModalBody())
	if !strings.Contains(body, "more") {
		t.Errorf("long list missing scroll indicator:\n%s", body)
	}
	t.Logf("LONG LIST:\n%s", body)
}

// keyMsg builds a tea.KeyPressMsg from its string form. v2 KeyPressMsg carries
// Code (the rune) and Text (the printed string); String() derives from Text.
func keyMsg(name string) tea.KeyPressMsg {
	r := []rune(name)
	if len(r) == 0 {
		return tea.KeyPressMsg{}
	}
	return tea.KeyPressMsg{Code: r[0], Text: name}
}

// TestFullView assembles the complete chrome (header + viewport + position bar
// + footer + input) and asserts every region is present, then logs the screen.
func TestFullView(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width = 120
	s.height = 22
	s.authed = true
	s.approvalModeStr = "destructive"
	s.turnCount = 3
	s.models = []modelInfo{{ID: "umans-glm-5.2", ContextWindow: 131072, MaxTokens: 8192}}
	s.modelIdx = 0
	s.contextTokens = 1480
	s.lastMetrics = []byte(`{"tps":"42.1","ttft_ms":"180"}`)
	s.layout()

	s.logUser("How do I read a file in Go?")
	b := s.push(blkAssistant)
	b.model = "umans-glm-5.2"
	b.text.WriteString("Use `os.ReadFile`:\n\n```go\ndata, err := os.ReadFile(\"path\")\n```\n")
	s.cur = nil
	s.invalidateAll()
	s.refresh()

	view := stripANSI(s.View().Content)
	for _, want := range []string{"Catalyst", "umans-glm-5.2", "ready", "How do I read a file", "os.ReadFile", "Enter send", "Chat with the agent"} {
		if !strings.Contains(view, want) {
			t.Errorf("full view missing %q:\n%s", want, view)
		}
	}
	t.Logf("FULL VIEW:\n%s", view)
}

// TestActiveTasks covers the unified activity shelf: routine activity is a
// single summary row, expands on demand, and clears when work finishes.
func TestActiveTasks(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width = 80
	s.height = 24
	s.authed = true
	s.models = []modelInfo{{ID: "umans-glm-5.2", ContextWindow: 131072}}
	s.modelIdx = 0
	s.layout()

	// A scout in flight: the core emits subagent_progress events that populate
	// s.subProgress (the panel's real data source), not the tool block itself.
	s.subProgress = append(s.subProgress, &subProgressEntry{
		runID: "call-1", agent: "scout", started: time.Now(),
		toolCount: 2, tokensIn: 100, tokensOut: 50,
		curTool: "read_file", toolRunning: true, toolStart: time.Now(),
	})
	s.busy = true
	s.layout() // simulate the tool_call handler making room for the panel

	panel := stripANSI(s.renderActivityShelf())
	if panel == "" {
		t.Fatal("activity shelf should render while a scout is in flight")
	}
	for _, want := range []string{"Subagents", "active", "expand"} {
		if !strings.Contains(panel, want) {
			t.Errorf("shelf missing %q:\n%s", want, panel)
		}
	}
	s.activityExpanded = true
	panel = stripANSI(s.renderActivityShelf())
	for _, want := range []string{"Activity", "Subagents", "scout", "read_file", "Esc close"} {
		if !strings.Contains(panel, want) {
			t.Errorf("expanded shelf missing %q:\n%s", want, panel)
		}
	}
	t.Logf("PANEL:\n%s", panel)

	// finalize the run → the panel clears
	s.subProgress = nil
	s.invalidateAll()
	if p := s.renderActivityShelf(); p != "" {
		t.Errorf("shelf should be empty once the run finishes, got:\n%s", p)
	}

	// composer: rounded card + prompted input row
	box := stripANSI(s.renderInputBox())
	lines := strings.Split(box, "\n")
	if len(lines) != 3 {
		t.Fatalf("composer should be 3 lines, got %d:\n%s", len(lines), box)
	}
	if !strings.HasPrefix(lines[0], "╭") || !strings.Contains(lines[1], "❯ ") {
		t.Errorf("composer missing card border or prompt:\n%s", box)
	}
	if !strings.Contains(lines[1], "Enter queues") {
		t.Errorf("busy input box missing in-flight placeholder:\n%s", box)
	}
	t.Logf("INPUTBOX:\n%s", box)
}

// TestSubToolCollapse verifies spawn:* sub-agent internal calls render as a
// compact dim one-liner (not a full card), so a scout's chatter stays tidy.
func TestSubToolCollapse(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width = 80
	s.height = 24
	s.models = []modelInfo{{ID: "umans-glm-5.2"}}
	s.modelIdx = 0
	s.layout()

	sub := s.logTool("read_file", `{"path":"src/main.rs"}`, true) // sub-agent internal call
	sub.dur = 200 * time.Millisecond
	sub.output = "line 1\nline 2"
	s.invalidateAll()
	s.refresh()

	rendered := stripANSI(s.renderBlock(sub, s.viewport.Width()))
	// collapsed: a dim one-liner with the name + args, NO result panel body
	if !strings.Contains(rendered, "read_file") || !strings.Contains(rendered, "src/main.rs") {
		t.Errorf("sub-tool line missing name/args:\n%s", rendered)
	}
	if strings.Contains(rendered, "line 1") {
		t.Errorf("sub-tool should collapse its result, but got output body:\n%s", rendered)
	}
	t.Logf("SUBTOOL:\n%s", rendered)
}

// TestToolOutputTruncation checks the default 3-line truncation and the
// ctrl+o expand path: collapsed shows the hint, expanded shows the full body.
func TestToolOutputTruncation(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.models = []modelInfo{{ID: "umans-glm-5.2"}}
	s.modelIdx = 0
	s.layout()
	tb := s.logTool("bash", `"ls"`, false)
	tb.output = "line1\nline2\nline3\nline4\nline5"
	tb.dur = 10 * time.Millisecond
	s.invalidateAll()

	collapsed := stripANSI(s.renderBlock(tb, s.viewport.Width()))
	if !strings.Contains(collapsed, "line3") {
		t.Errorf("collapsed should show first 3 lines:\n%s", collapsed)
	}
	if strings.Contains(collapsed, "line5") {
		t.Errorf("collapsed should hide line5:\n%s", collapsed)
	}
	if !strings.Contains(collapsed, "ctrl+o expand") {
		t.Errorf("collapsed should show expand hint:\n%s", collapsed)
	}

	tb.expanded = true
	expanded := stripANSI(s.renderBlock(tb, s.viewport.Width()))
	if !strings.Contains(expanded, "line5") {
		t.Errorf("expanded should show full output:\n%s", expanded)
	}
	if !strings.Contains(expanded, "ctrl+o collapse") {
		t.Errorf("expanded should show collapse hint:\n%s", expanded)
	}

	if s.lastToolOutputBlock() != tb {
		t.Errorf("lastToolOutputBlock should find the tool block")
	}
}

func TestToolActivityGroupsCallsAndPreservesDetails(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 88, 28
	s.models = []modelInfo{{ID: "test-model"}}
	s.modelIdx = 0
	s.layout()

	s.logUser("Inspect, update, and verify the parser")
	read := s.logTool("read_file", `{"path":"parser.go","limit":"40"}`, false)
	read.output, read.hasOk, read.ok, read.dur = "package parser\n\nfunc parse() {}", true, true, 20*time.Millisecond
	edit := s.logTool("edit", `{"path":"parser.go","edits":[{"old_text":"parse","new_text":"Parse"}]}`, false)
	edit.output, edit.hasOk, edit.ok, edit.dur = "updated parser.go", true, true, 30*time.Millisecond
	test := s.logTool("bash", `{"command":"go test ./..."}`, false)
	test.output, test.hasOk, test.ok, test.dur = "ok parser", true, true, 40*time.Millisecond
	s.push(blkAssistant).appendText("The parser is updated and the tests pass.")
	s.cur = nil
	s.invalidateAll()

	compact := stripANSI(s.renderBlocks())
	if strings.Count(compact, "▸ activity") != 1 {
		t.Fatalf("consecutive tools should share one activity heading:\n%s", compact)
	}
	for _, want := range []string{"3 calls", "parser.go", "1 replacement", "go test ./...", "The parser is updated"} {
		if !strings.Contains(compact, want) {
			t.Errorf("compact activity missing %q:\n%s", want, compact)
		}
	}
	for _, hidden := range []string{"package parser", "updated parser.go", "ok parser"} {
		if strings.Contains(compact, hidden) {
			t.Errorf("compact activity leaked output %q:\n%s", hidden, compact)
		}
	}

	edit.expanded = true
	s.invalidateAll()
	expanded := stripANSI(s.renderBlocks())
	if !strings.Contains(expanded, "updated parser.go") {
		t.Fatalf("expanded call should reveal its tool-specific output:\n%s", expanded)
	}
	if strings.Contains(expanded, "package parser") || strings.Contains(expanded, "ok parser") {
		t.Fatalf("expanding one call should leave sibling calls compact:\n%s", expanded)
	}
	if edit.renderEnd <= edit.renderStart {
		t.Fatalf("expanded call should own a multi-line navigation range: %d-%d", edit.renderStart, edit.renderEnd)
	}
}
