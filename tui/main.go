package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/charmbracelet/bubbles/spinner"
	"github.com/charmbracelet/bubbles/textinput"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

// session is the Bubble Tea model: it owns the core subprocess, the structured
// list of message blocks, and the viewport/input/spinner.
type session struct {
	coreCmd    *exec.Cmd
	coreIn     io.WriteCloser
	coreEvents chan *coreEvent

	authed              bool
	models              []modelInfo
	modelIdx            int
	busy                bool
	queuedNext          bool // a follow-up/steer turn is chained after the current one
	turnCount           int
	pendingApproval     *approvalPrompt
	pendingIntercom     *intercomPrompt
	lastMetrics         json.RawMessage
	approvalModeStr     string
	sessionList         []sessionEntry
	coreBashTimeout     int
	coreRestarts        int
	visionModels        map[string]bool // user-curated vision-capable model ids (drives /vision)
	visionModel         string          // preferred handoff target ("" = pick dynamically)
	pendingVisionPicker bool            // open the vision picker once the config arrives

	settings *settingsStore
	modal    modal
	history  []string
	histIdx  int

	// conversation blocks + streaming/caching state
	blocks        []*block
	cur           *block // currently-streaming block (assistant/thinking), or nil
	thinkExpanded bool   // global reasoning expand state (default collapsed)
	cache         strings.Builder
	cacheIdx      int

	// scroll: follow=true keeps the viewport pinned to the newest line (the
	// default). Scrolling up pauses follow so history can be read without the
	// view yanking back down on each new token; a banner offers to re-pin.
	follow        bool
	welcomeIdx    int                 // welcome-screen example cursor (empty conversation)
	contextTokens uint64              // live context size from the last metrics event (drives the footer budget)
	subProgress   []*subProgressEntry // live subagent runs (drives the progress panel)
	cwd           string              // working dir, shown in the header as ~/

	// @-mention file flyout state (see mention.go): active when an
	// unbroken @-token sits at the cursor; mentionAt is its rune index.
	mentionActive bool
	mentionItems  []mentionItem
	mentionCursor int
	mentionScroll int
	mentionAt     int

	viewport viewport.Model
	input    textinput.Model
	spinner  spinner.Model

	width, height int
	ready         bool
}

func initialSession() *session {
	s := &session{}

	s.settings = loadSettings()
	if s.settings.Theme != "" {
		setTheme(s.settings.Theme)
	}
	s.thinkExpanded = s.settings.ThinkExpanded
	s.follow = true // pin viewport to newest line until the user scrolls up
	s.cwd = cwdDisplay()
	s.coreBashTimeout = 30
	s.visionModels = map[string]bool{}

	s.input = textinput.New()
	s.input.Placeholder = "Chat with the agent…  (/ for commands)"
	s.input.PlaceholderStyle = placeholderStyle
	s.input.Prompt = ""
	s.input.Focus()

	s.viewport = viewport.New(80, 20)
	s.viewport.SetContent("")

	sp := spinner.New()
	sp.Spinner = spinner.Dot
	sp.Style = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent))
	s.spinner = sp

	return s
}

// ---------------------------------------------------------------------------
// Core subprocess lifecycle
// ---------------------------------------------------------------------------

// coreExeSuffix returns the platform executable suffix (".exe" on Windows,
// "" elsewhere) so the installed Windows layout (ucli.exe + umans-core.exe)
// is discovered correctly.
func coreExeSuffix() string {
	if runtime.GOOS == "windows" {
		return ".exe"
	}
	return ""
}

// coreBinaryPath resolves the core subprocess binary. Search order:
//  1. $UMANS_CORE — explicit override (used as-is if it exists)
//  2. <dir of this exe>/umans-core[.exe] — installed layout (beside the TUI)
//  3. <dir of this exe>/core[.exe] — dev/legacy core placed beside the TUI
//  4. core/target/release/core[.exe] — dev build run from the repo root
//  5. ../core/target/release/core[.exe] — dev build run from a sibling dir
//  6. <dir of this exe>/../core/target/release/core[.exe]
//
// On Windows ".exe" is appended to every candidate so the install layout
// (ucli.exe next to umans-core.exe) is found from any CWD.
func coreBinaryPath() string {
	if env := os.Getenv("UMANS_CORE"); env != "" {
		if _, err := os.Stat(env); err == nil {
			if abs, err := filepath.Abs(env); err == nil {
				return abs
			}
			return env
		}
	}
	if p := embeddedCorePath(); p != "" {
		return p
	}
	sfx := coreExeSuffix()
	coreName := "umans-core" + sfx // installed beside the TUI
	devName := "core" + sfx        // cargo's bin name in the dev build
	candidates := []string{
		"core/target/release/" + devName,
		"../core/target/release/" + devName,
	}
	if exe, err := os.Executable(); err == nil {
		dir := filepath.Dir(exe)
		candidates = append(candidates,
			filepath.Join(dir, coreName), // installed layout
			filepath.Join(dir, devName),  // dev/legacy core beside the TUI
			filepath.Join(dir, "../core/target/release/"+devName),
		)
	}
	for _, c := range candidates {
		if _, err := os.Stat(c); err == nil {
			if abs, err := filepath.Abs(c); err == nil {
				return abs
			}
			return c
		}
	}
	return "core/target/release/" + devName
}

func (s *session) startCore() tea.Cmd {
	bin := coreBinaryPath()
	approval := s.settings.Approval
	if approval == "" {
		approval = "destructive"
	}
	args := []string{
		"--workspace", ".",
		"--approval", approval,
		"--session", sessionPath(),
		"--debug-log", filepath.Join(configDir(), "debug.jsonl"),
		"--idle-timeout", fmt.Sprintf("%d", s.settings.IdleTimeout),
	}
	if s.settings.Sandbox != "" && s.settings.Sandbox != "none" {
		args = append(args, "--sandbox", s.settings.Sandbox)
	}
	if s.settings.NoNetwork {
		args = append(args, "--no-network")
	}
	if s.settings.MaxSessionTokens > 0 {
		args = append(args, "--max-session-tokens", fmt.Sprintf("%d", s.settings.MaxSessionTokens))
	}
	cmd := exec.Command(bin, args...)
	cmd.Dir, _ = os.Getwd()
	in, err := cmd.StdinPipe()
	if err != nil {
		s.logError("failed to open core stdin: " + err.Error())
		return nil
	}
	out, err := cmd.StdoutPipe()
	if err != nil {
		s.logError("failed to open core stdout: " + err.Error())
		return nil
	}
	cmd.Stderr = os.Stderr
	if err := cmd.Start(); err != nil {
		s.logError("failed to start core (" + bin + "): " + err.Error())
		return nil
	}
	s.coreCmd = cmd
	s.coreIn = in
	s.coreEvents = make(chan *coreEvent, 256)

	go func() {
		sc := bufio.NewScanner(out)
		sc.Buffer(make([]byte, 0, 64*1024), 4*1024*1024)
		for sc.Scan() {
			line := strings.TrimSpace(sc.Text())
			if line == "" {
				continue
			}
			var raw json.RawMessage
			var ev coreEvent
			if err := json.Unmarshal([]byte(line), &raw); err == nil {
				ev.Raw = raw
				if t := ev.get("type"); t != "" {
					ev.Type = t
				}
			}
			select {
			case s.coreEvents <- &ev:
			default:
			}
		}
		close(s.coreEvents)
	}()

	s.sendCore(map[string]any{"type": "init"})
	return waitForEvent(s.coreEvents)
}

func waitForEvent(ch <-chan *coreEvent) tea.Cmd {
	return func() tea.Msg {
		ev, ok := <-ch
		if !ok {
			return coreEOFMsg{}
		}
		return coreEventMsg{ev}
	}
}

func (s *session) sendCore(m map[string]any) {
	if s.coreIn == nil {
		return
	}
	b, _ := json.Marshal(m)
	b = append(b, '\n')
	_, _ = s.coreIn.Write(b)
}

// ---------------------------------------------------------------------------
// Tea Model methods
// ---------------------------------------------------------------------------

func (s *session) Init() tea.Cmd {
	return tea.Batch(s.startCore(), tick(), s.spinner.Tick)
}

func tick() tea.Cmd {
	return tea.Tick(time.Second, func(t time.Time) tea.Msg { return tickMsg{t} })
}

func (s *session) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		s.width = msg.Width
		s.height = msg.Height
		s.ready = true
		s.layout()
		return s, nil

	case tickMsg:
		return s, tick()

	case spinner.TickMsg:
		var cmd tea.Cmd
		s.spinner, cmd = s.spinner.Update(msg)
		return s, cmd

	case coreEOFMsg:
		// Core crashed or exited unexpectedly. Auto-restart once so the user
		// isn't stranded, and re-auth with the persisted key if we had one.
		// ponytail: one retry. A crash loop would mean a real bug to fix, not
		// silent infinite restarts.
		if s.coreRestarts >= 1 {
			s.logError("core exited again after restart; quitting")
			return s, tea.Quit
		}
		s.coreRestarts++
		s.logWarn("core exited; restarting…")
		s.coreCmd = nil
		s.coreIn = nil
		return s, s.startCore()

	case coreEventMsg:
		return s, s.handleCoreEvent(msg.event)

	case tea.KeyMsg:
		model, cmd := s.handleKey(msg)
		return model, cmd

	case tea.MouseMsg:
		return s, s.handleMouseWheel(msg)

	default:
		// bubbletea v1.3 can't decode modified-Enter (the Key type carries no
		// modifier bits), so terminals send Ctrl+Enter as an unrecognized CSI sequence.
		// Intercept it here to honor the steer binding; terminals that send a plain
		// CR for Ctrl+Enter instead receive it as a normal "enter" (follow-up).
		if s.modal.kind == modalNone && isCtrlEnterUnknownCSI(msg) {
			return s, s.steerFromInput()
		}
	}
	return s, nil
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

func main() {
	opts := []tea.ProgramOption{tea.WithAltScreen()}
	if loadSettings().MouseWheel {
		opts = append(opts, tea.WithMouseCellMotion())
	}
	prog := tea.NewProgram(initialSession(), opts...)
	if _, err := prog.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}
