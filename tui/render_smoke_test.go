package main

import (
	"fmt"
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
)

func stripANSI(s string) string {
	var b strings.Builder
	in := false
	for _, r := range s {
		if r == 0x1b {
			in = true
			continue
		}
		if in {
			if r == 'm' || r == 'H' || r == 'J' || r == 'K' || r == '[' || r == ';' || r == '?' || (r >= '0' && r <= '9') {
				continue
			}
			in = false
		}
		b.WriteRune(r)
	}
	return b.String()
}

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

	// welcome screen: brand + a selectable example panel
	welcome := stripANSI(s.renderBlocks())
	for _, want := range []string{"Umans", "Examples", "Explain"} {
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
	tb.output = "ok  umans-harness-tui  0.012s\nPASS"
	tb.dur = 1400000000
	s.invalidateAll()

	s.push(blkError).text.WriteString("not authenticated — run /key sk-... first")
	s.invalidateAll()

	// flat layout: role glyphs + content, tool output in a │ panel
	blocks := stripANSI(s.renderBlocks())
	for _, want := range []string{"you", "leader", "bash", "go test", "PASS", "✗", "not authenticated"} {
		if !strings.Contains(blocks, want) {
			t.Errorf("blocks missing %q:\n%s", want, blocks)
		}
	}
	t.Logf("BLOCKS:\n%s", blocks)

	// header: brand + tagline / cwd + tip (model & approval moved to footer)
	hdr := stripANSI(s.renderHeader())
	for _, want := range []string{"Umans", "Tip"} {
		if !strings.Contains(hdr, want) {
			t.Errorf("header missing %q:\n%s", want, hdr)
		}
	}
	t.Logf("HEADER:\n%s", hdr)

	// footer: ready state · leader · model · approval · context budget
	ftr := stripANSI(s.renderFooter())
	for _, want := range []string{"ready", "leader", "umans-glm-5.2", "destructive"} {
		if !strings.Contains(ftr, want) {
			t.Errorf("footer missing %q:\n%s", want, ftr)
		}
	}
	t.Logf("FOOTER:\n%s", ftr)

	// position bar: pinned to bottom shows a 100% rule (no new-lines affordance)
	pos := stripANSI(s.renderPositionBar())
	if !strings.Contains(pos, "100%") {
		t.Errorf("position bar missing 100%% when pinned:\n%s", pos)
	}
	t.Logf("POSITION:\n%s", pos)

	s.pendingApproval = &approvalPrompt{requestID: "r1", tool: "bash", args: "rm -rf /tmp/cache"}
	banner := stripANSI(s.renderApprovalBanner())
	if !strings.Contains(banner, "approve") || !strings.Contains(banner, "bash") {
		t.Errorf("approval banner missing content:\n%s", banner)
	}
	t.Logf("BANNER:\n%s", banner)

	s.openCommandPalette()
	pal := stripANSI(s.renderModalBody())
	if !strings.Contains(pal, "Command Palette") || !strings.Contains(pal, "model") {
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
	yoff := s.viewport.YOffset
	s.logInfo("a new line arrives while reading")
	if s.viewport.YOffset != yoff {
		t.Errorf("view yanked while paused: was %d now %d", yoff, s.viewport.YOffset)
	}
}

func TestMarkdown(t *testing.T) {
	out := stripANSI(renderMarkdown("Here is `inline` code and **bold** and *italic*.\n\n```go\nch := make(chan int)\n```", 60))
	for _, want := range []string{"inline", "bold", "italic", "go", "ch := make(chan int)"} {
		if !strings.Contains(out, want) {
			t.Errorf("markdown missing %q:\n%s", want, out)
		}
	}
	t.Logf("MARKDOWN:\n%s", out)

	// code block never reflows; over-long lines truncate
	long := renderMarkdown("```sh\n"+strings.Repeat("x", 200)+"\n```", 40)
	if !strings.Contains(long, "│") {
		t.Errorf("code block missing left rule:\n%s", long)
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
		if !strings.Contains(out, "Umans") {
			t.Errorf("theme %q header broke: %s", name, out)
		}
	}
	setTheme("mocha")

	// metrics: chrome shows throughput (tps) only; ttft + cumulative tokens
	// live in /stats + the debug log. The context budget is separate.
	s.lastMetrics = []byte(`{"tps":"42.1","ttft_ms":"180","tokens_in":"1000","tokens_out":"200"}`)
	s.contextTokens = 1200
	m := s.renderMetrics()
	for _, w := range []string{"tok/s", "42.1"} {
		if !strings.Contains(m, w) {
			t.Errorf("metrics missing %q in %q", w, m)
		}
	}
	if strings.Contains(m, "ttft") {
		t.Errorf("chrome metrics should omit ttft, got %q", m)
	}
	// context budget: cumulative tokens / model window → "15% 1.2k/8.2k"
	ctx := s.renderContext()
	for _, w := range []string{"%", "1.2k", "8.2k"} {
		if !strings.Contains(ctx, w) {
			t.Errorf("context missing %q in %q", w, ctx)
		}
	}
	t.Logf("METRICS: %s  CONTEXT: %s", m, ctx)

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

// keyMsg builds a tea.KeyMsg from its string form (tea.KeyType defaults to
// KeyRunes so the String() matches).
func keyMsg(name string) tea.KeyMsg {
	return tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune(name)}
}

// TestFullView assembles the complete chrome (header + viewport + position bar
// + footer + input) and asserts every region is present, then logs the screen.
func TestFullView(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width = 92
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

	view := stripANSI(s.View())
	for _, want := range []string{"Umans", "umans-glm-5.2", "ready", "you", "leader", "os.ReadFile", "tok/s", "Chat with the agent"} {
		if !strings.Contains(view, want) {
			t.Errorf("full view missing %q:\n%s", want, view)
		}
	}
	t.Logf("FULL VIEW:\n%s", view)
}

// TestActiveTasks covers the active-tasks panel: an in-flight spawn (scout)
// renders a bordered panel with the scout's task, role and model; finalizing
// the tool clears it. Also checks the bordered input box renders.
func TestActiveTasks(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width = 80
	s.height = 24
	s.authed = true
	s.models = []modelInfo{{ID: "umans-glm-5.2", ContextWindow: 131072}}
	s.modelIdx = 0
	s.layout()

	// a scout in flight (spawn, no result yet)
	scout := s.logTool("spawn", `{"prompt":"Explore the core package","model":"umans-glm-5.2"}`, false)
	scout.id = "call-1"
	s.busy = true
	s.layout() // simulate the tool_call handler making room for the panel

	panel := stripANSI(s.renderActiveTasks(s.width))
	if panel == "" {
		t.Fatal("active-tasks panel should render while a scout is in flight")
	}
	for _, want := range []string{"active tasks", "scout", "Explore the core package", "umans-glm-5.2"} {
		if !strings.Contains(panel, want) {
			t.Errorf("panel missing %q:\n%s", want, panel)
		}
	}
	if h := s.activeTasksHeight(); h == 0 {
		t.Error("activeTasksHeight should be > 0 while a scout is in flight")
	}
	t.Logf("PANEL:\n%s", panel)

	// finalize the scout → the panel clears
	scout.output = "the core package is an async agent loop…"
	scout.dur = 5 * time.Second
	s.invalidateAll()
	if p := s.renderActiveTasks(s.width); p != "" {
		t.Errorf("panel should be empty once the scout finishes, got:\n%s", p)
	}
	if h := s.activeTasksHeight(); h != 0 {
		t.Errorf("activeTasksHeight should be 0 once finished, got %d", h)
	}

	// bordered input box: top + input row + bottom
	box := stripANSI(s.renderInputBox())
	lines := strings.Split(box, "\n")
	if len(lines) != 3 {
		t.Fatalf("input box should be 3 lines, got %d:\n%s", len(lines), box)
	}
	if !strings.HasPrefix(lines[0], "╭") || !strings.HasPrefix(lines[2], "╰") {
		t.Errorf("input box missing rounded borders:\n%s", box)
	}
	if !strings.Contains(lines[1], "Chat with the agent") {
		t.Errorf("input row missing placeholder:\n%s", box)
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

	rendered := stripANSI(s.renderBlock(sub, s.viewport.Width))
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

	collapsed := stripANSI(s.renderBlock(tb, s.viewport.Width))
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
	expanded := stripANSI(s.renderBlock(tb, s.viewport.Width))
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
