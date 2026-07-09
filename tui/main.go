package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"runtime"
	"strings"
	"sync/atomic"
	"syscall"
	"time"

	"charm.land/bubbles/v2/spinner"
	"charm.land/bubbles/v2/textinput"
	"charm.land/bubbles/v2/viewport"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

// queuedMsg is the single buffered follow-up/steer prompt shown in a pinned
// banner while a turn runs. kind is "follow-up" (queued via Enter) or "steer"
// (Ctrl+Enter / /steer). Cleared when the queued turn starts, when it is
// dequeued via Esc, or on full abort.
type queuedMsg struct {
	kind string // "follow-up" | "steer"
	text string
	at   time.Time
}

// session is the Bubble Tea model: it owns the core subprocess, the structured
// list of message blocks, and the viewport/input/spinner.
type session struct {
	coreCmd    *exec.Cmd
	coreIn     io.WriteCloser
	coreEvents chan *coreEvent
	stdinCh    chan []byte // P1-15: stdin writes are funneled through a writer goroutine

	authed              bool
	models              []modelInfo
	modelIdx            int
	busy                bool
	queuedNext          bool                         // a follow-up/steer turn is chained after the current one
	queued              *queuedMsg                   // the currently-queued follow-up/steer (drives the pinned banner + Esc-dequeue)
	todos               []map[string]json.RawMessage // latest todo_write state (pinned panel)
	turnCount           int
	pendingApproval     *approvalPrompt
	pendingIntercom     *intercomPrompt
	intercomNudge       time.Time // pulses a "type a reply" hint when Enter is hit on an empty intercom reply
	pendingAsk          *askPrompt
	updateInfo          *updateInfo // non-nil when a newer release is available (drives the top banner)
	lastMetrics         json.RawMessage
	approvalModeStr     string
	sessionList         []sessionEntry
	skillsList          []skillInfo // discoverable skills (drives /skill:<name> autocomplete)
	coreBashTimeout     int
	coreAutoCompact     bool
	ctxBreakdown        *contextBreakdown
	coreRestarts        int
	coreReady           bool            // true once the core emitted `ready` (disarms the startup watchdog)
	coreStartGen        uint64          // bumped each startCore; lets a stale watchdog tick ignore a restart
	visionModels        map[string]bool // user-curated vision-capable model ids (drives /vision)
	visionModel         string          // preferred handoff target ("" = pick dynamically)
	pendingVisionPicker bool            // open the vision picker once the config arrives

	// Active model provider (openai/anthropic endpoint). activeProvider is the
	// name the core resolved; providers is the list of configured names for the
	// settings picker; providerHasKey reflects the last provider_changed/ready
	// event (drives re-auth after a switch).
	activeProvider  string
	providerKind    string
	providers       []string
	providerHasKey  bool
	providerPresets []providerPreset
	pendingLogin    string // preset id awaiting a pasted API key in the /login modal

	settings *settingsStore
	keybinds map[string]string // effective keymap (defaults + user overrides); see keybinds.go
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
	follow            bool
	welcomeIdx        int                 // welcome-screen example cursor (empty conversation)
	contextTokens     uint64              // live context size from the last metrics event (drives the footer budget)
	lastCachePct      int                 // last completed turn's prefix-cache hit %; shown (with "~") while the next turn is in flight
	tokensSaved       uint64              // cumulative tokens reclaimed by digest + compaction (shown next to "cached" in the footer)
	summaryChars      int                 // character count of the current rolling compaction summary (0 until a summary is produced)
	umansConcUsed     *int64              // live Umans account-wide concurrency in use; nil => not Umans / fetch failed (hide the field)
	umansConcLimit    *int64              // Umans plan concurrency ceiling; nil => unlimited (render ∞); only meaningful when used != nil
	umansConcProvider string              // the Umans provider name the poll is tracking; conc shows only when the selected model routes here
	subProgress       []*subProgressEntry // live subagent runs (drives the progress panel)
	maxTaskRows       int                 // cap on task-panel entries (set by layout() to fit available height)
	cwd               string              // working dir, shown in the header as ~/

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
	spinnerActive bool // whether the spinner animation cycle is running (stopped when idle to avoid re-render storms that disrupt text selection)

	width, height int
	ready         bool
}

func initialSession() *session {
	s := &session{}

	s.settings = loadSettings()
	s.keybinds = effectiveKeybinds(s.settings.Keybinds)
	if s.settings.Theme != "" {
		setTheme(s.settings.Theme)
	}
	s.thinkExpanded = s.settings.ThinkExpanded
	s.follow = true // pin viewport to newest line until the user scrolls up
	s.cwd = cwdDisplay()
	s.maxTaskRows = 4 // cap on task-panel entries; layout() shrinks it to fit available height
	s.coreBashTimeout = 30
	s.visionModels = map[string]bool{}

	s.input = textinput.New()
	s.input.Placeholder = "Chat with the agent…  (/ for commands)"
	// textinput v2: placeholder style lives on Styles().{Focused,Blurred}.Placeholder
	// (the top-level PlaceholderStyle field was removed).
	ist := s.input.Styles()
	ist.Focused.Placeholder = placeholderStyle
	ist.Blurred.Placeholder = placeholderStyle
	s.input.SetStyles(ist)
	s.input.Prompt = ""
	s.input.Focus()
	s.enableMultilineInput() // keep typed/pasted newlines (see extras.go)

	s.viewport = viewport.New(viewport.WithWidth(80), viewport.WithHeight(20))
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
// "" elsewhere) so the installed Windows layout (catcode.exe + catcode-core.exe)
// is discovered correctly.
func coreExeSuffix() string {
	if runtime.GOOS == "windows" {
		return ".exe"
	}
	return ""
}

// coreBinaryPath resolves the core subprocess binary. Search order:
//  1. $CATCODE_CORE — explicit override (used as-is if it exists)
//  2. <dir of this exe>/catcode-core[.exe] — installed layout (beside the TUI)
//  3. <dir of this exe>/core[.exe] — dev/legacy core placed beside the TUI
//  4. core/target/release/core[.exe] — dev build run from the repo root
//  5. ../core/target/release/core[.exe] — dev build run from a sibling dir
//  6. <dir of this exe>/../core/target/release/core[.exe]
//
// On Windows ".exe" is appended to every candidate so the install layout
// (catcode.exe next to catcode-core.exe) is found from any CWD.
func coreBinaryPath() string {
	if env := os.Getenv("CATCODE_CORE"); env != "" {
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
	coreName := "catcode-core" + sfx // installed beside the TUI
	devName := "core" + sfx          // cargo's bin name in the dev build
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

// coreProcess holds the running core's *os.Process so a signal handler can
// kill it on SIGHUP/SIGTERM — otherwise closing the terminal (SIGHUP) or `kill`
// (SIGTERM) kills the TUI but orphans catcode-core, which keeps running.
// Set in startCore after cmd.Start() (UI thread); read from the signal-handler
// goroutine. An atomic.Pointer is used because the field is shared across
// goroutines — a plain var would be a data race.
var coreProcess atomic.Pointer[os.Process]

// quitting is set by the signal handler / quit key before killing the core, so
// the coreEOFMsg auto-restart path doesn't spawn a fresh core while the TUI is
// tearing down (a killed core's stdout EOF would otherwise look like a crash).
var quitting atomic.Bool

func (s *session) startCore() tea.Cmd {
	// Reset startup-tracking state and arm a fresh watchdog generation so a stale
	// watchdog tick from a previous (crashed) core is ignored once `ready` lands.
	s.coreReady = false
	s.coreStartGen++
	gen := s.coreStartGen
	bin := coreBinaryPath()
	approval := s.settings.Approval
	if approval == "" {
		approval = "destructive"
	}
	debugLog := filepath.Join(configDir(), "debug.jsonl")
	args := []string{
		"--workspace", ".",
		"--approval", approval,
		"--session", sessionPath(),
		"--debug-log", debugLog,
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
	// P1-14: don't mutate UI state (logError) from this cmd goroutine on the
	// error paths — return a message and let Update log it on the UI thread.
	in, err := cmd.StdinPipe()
	if err != nil {
		return func() tea.Msg { return coreStartErrorMsg{fmt.Errorf("failed to open core stdin: %s", err)} }
	}
	out, err := cmd.StdoutPipe()
	if err != nil {
		return func() tea.Msg { return coreStartErrorMsg{fmt.Errorf("failed to open core stdout: %s", err)} }
	}
	// P2: route the core's stderr (panic backtraces, unexpected warnings) to the
	// debug log instead of the terminal — under the alt-screen TUI, raw stderr is
	// buffered/lost and garbles the screen. The core appends its structured logs
	// to the same file, so this is safe (append, no truncate race).
	if f, ferr := os.OpenFile(debugLog, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600); ferr == nil {
		cmd.Stderr = f
		defer f.Close() // child inherits the fd after Start; close our copy
	} else {
		cmd.Stderr = os.Stderr
	}
	if err := cmd.Start(); err != nil {
		return func() tea.Msg { return coreStartErrorMsg{fmt.Errorf("failed to start core (%s): %s", bin, err)} }
	}
	s.coreCmd = cmd
	coreProcess.Store(cmd.Process) // expose to the signal handler (M8): kill on SIGHUP/SIGTERM
	s.coreIn = in
	s.coreEvents = make(chan *coreEvent, 256)
	s.stdinCh = make(chan []byte, 256)

	// P1-15: a dedicated stdin-writer goroutine funnels commands to the core. A
	// blocking pipe write (core not draining) happens here, off the UI thread,
	// so a wedged core can never freeze the Bubble Tea Update loop. The locals
	// (in/ch) are captured once so the writer never reads the shared s.coreIn /
	// s.stdinCh fields, which the restart path nils concurrently — avoids a data
	// race / nil-deref when the core is restarted mid-write.
	ch := s.stdinCh
	go func() {
		for b := range ch {
			if _, err := in.Write(b); err != nil {
				return // core died; the stdout EOF will trigger restart
			}
		}
	}()

	// P1-11: bufio.Reader.ReadString grows with the line (no 4 MiB cap like
	// bufio.Scanner), so a large tool_result JSON line doesn't silently kill the
	// stream. P1-10: the send is BLOCKING (backpressure) so a `done` or
	// `approval_request` is never silently dropped — the core's stdout pipe fills
	// and naturally throttles the core instead.
	go func() {
		r := bufio.NewReaderSize(out, 64*1024)
		for {
			line, err := r.ReadString('\n')
			line = strings.TrimSpace(line)
			if line != "" {
				var raw json.RawMessage
				var ev coreEvent
				if json.Unmarshal([]byte(line), &raw) == nil {
					ev.Raw = raw
					if t := ev.get("type"); t != "" {
						ev.Type = t
					}
				}
				// Blocking send preserves backpressure (a `done` or
				// approval_request is never silently dropped). But when the read
				// also returned an error (io.EOF at core exit), the UI may have
				// stopped draining — a blocking send here would wedge the reader
				// forever, skipping cmd.Wait()/close() and leaking the core as a
				// zombie. Drop the final line non-blockingly on error so we reach
				// the break and reap the child.
				if err != nil {
					select {
					case s.coreEvents <- &ev:
					default:
					}
				} else {
					s.coreEvents <- &ev
				}
			}
			if err != nil {
				if err != io.EOF {
					// Surface a real read error instead of a silent clean-looking EOF.
					ev := &coreEvent{Type: "error"}
					msg, _ := json.Marshal(map[string]string{"message": "core stdout read error: " + err.Error()})
					ev.Raw = msg
					select {
					case s.coreEvents <- ev:
					default:
					}
				}
				break
			}
		}
		// Reap the core child process exactly once, after the stdout pipe has
		// been fully read. Per os/exec: Wait must not be called before the pipe
		// drain completes — the reader just hit EOF, so the process has exited
		// and Wait only collects its status. This prevents the core from lingering
		// as a zombie across the crash auto-restart (which spawns a fresh core
		// each time) and on Ctrl+C quit. Uses the local cmd (not s.coreCmd, which
		// is nilled during restart) to avoid racing the recovery handler.
		_ = cmd.Wait()
		close(s.coreEvents)
	}()

	s.sendCore(map[string]any{"type": "init"})
	// Arm a startup watchdog: if the core starts but never emits `ready` within
	// coreStartupTimeout (e.g. a bad UMANS_CORE path or a config that panics),
	// surface a clear error instead of spinning "starting core…" forever. The
	// tick carries the generation captured above so a tick from a previous
	// (crashed+restarted) core is ignored once `ready` disarms it.
	return tea.Batch(
		waitForEvent(s.coreEvents),
		tea.Tick(coreStartupTimeout, func(time.Time) tea.Msg { return readyTimeoutMsg{gen: gen} }),
	)
}

// coreStartErrorMsg reports a core subprocess start failure (P1-14: logged on
// the UI thread, not from the startCore goroutine).
type coreStartErrorMsg struct{ err error }

// coreStartupTimeout is how long startCore's watchdog waits for a `ready` event
// before declaring the core failed to start.
const coreStartupTimeout = 30 * time.Second

// readyTimeoutMsg is delivered by the startup watchdog when the core has not
// emitted `ready` within coreStartupTimeout. gen ties it to a specific start so
// a tick from a previous (restarted) core is ignored.
type readyTimeoutMsg struct{ gen uint64 }

// sigtermMsg is sent by the SIGHUP/SIGTERM handler so Bubble Tea restores the
// terminal (alt-screen / raw-mode) via its normal tea.Quit path instead of a
// raw os.Exit that would leave the terminal broken.
type sigtermMsg struct{}

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
	// P1-15: hand the bytes to the stdin-writer goroutine instead of a direct
	// (possibly blocking) pipe write from the UI thread. Non-blocking send with a
	// drop+log on overflow so a wedged core can never freeze Update; in practice
	// the buffer never fills (commands are human-paced and the writer drains to
	// the pipe as fast as the core accepts).
	if s.stdinCh != nil {
		select {
		case s.stdinCh <- b:
		default:
			s.logError("core not accepting input (backpressure); command dropped")
		}
		return
	}
	_, _ = s.coreIn.Write(b) // legacy path before startCore wires stdinCh
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
		// Refresh the transcript while there's live content (a streaming block or
		// an in-flight tool) so the in-flight badge `◷` and its elapsed timer tick.
		// Finalized blocks are cached, so this is O(live) when idle it's a no-op.
		if s.hasLiveContent() {
			s.refresh()
		}
		cmds := []tea.Cmd{tick()}
		// (Re)start the spinner animation if a turn is in flight but the spinner
		// cycle has stopped. The cycle stops when idle (see spinner.TickMsg) to
		// avoid a ~20x/sec re-render storm that disrupts mouse text selection
		// (copy); tickMsg (every 500ms) catches the busy transition and restarts it.
		if (s.busy || !s.ready) && !s.spinnerActive {
			s.spinnerActive = true
			cmds = append(cmds, s.spinner.Tick)
		}
		return s, tea.Batch(cmds...)

	case spinner.TickMsg:
		s.spinner, _ = s.spinner.Update(msg)
		// Only keep the spinner animating while it's actually shown (a running turn
		// or still starting). When idle, stop ticking so the cursed renderer isn't
		// driven ~20x/sec — that constant re-render makes mouse text selection
		// (copy) impossible. tickMsg restarts the cycle when activity resumes.
		if s.busy || !s.ready {
			s.spinnerActive = true
			return s, s.spinner.Tick
		}
		s.spinnerActive = false
		return s, nil

	case coreStartErrorMsg:
		// P1-14: a core start failure is logged on the UI thread (startCore ran in
		// a goroutine and must not touch UI state itself).
		s.logError(msg.err.Error())
		return s, nil

	case updateAvailableMsg:
		// A newer GitHub release was found by the launch-time check. Store it so
		// renderUpdateBanner shows a one-line notice; re-layout to claim the line.
		s.updateInfo = &msg.info
		s.layout()
		return s, nil

	case readyTimeoutMsg:
		// The startup watchdog fired. Ignore if `ready` already arrived or this
		// tick belongs to a previous (restarted) core; otherwise the core never
		// came up — surface a clear error instead of spinning forever.
		if s.coreReady || msg.gen != s.coreStartGen {
			return s, nil
		}
		s.logError("core did not start within 30s — check UMANS_CORE path / config (Ctrl+C to quit)")
		return s, nil

	case sigtermMsg:
		// SIGHUP/SIGTERM: restore the terminal via the normal quit path (the
		// signal goroutine already killed the core; the reader reaps it).
		return s, tea.Quit

	case coreEOFMsg:
		// A signal-driven teardown (SIGHUP/SIGTERM) or the quit key killed the
		// core; the reader then reports EOF. Don't auto-restart — we're quitting.
		if quitting.Load() {
			return s, tea.Quit
		}
		// Core crashed or exited unexpectedly. Auto-restart once so the user
		// isn't stranded, and re-auth with the persisted key if we had one.
		// P1-17: coreRestarts is reset to 0 after every successful turn (see the
		// `done` handler), so this budget is per-incident, not lifetime — an early
		// crash doesn't burn the only retry forever.
		if s.coreRestarts >= 1 {
			s.logError("core exited again after restart; quitting")
			return s, tea.Quit
		}
		s.coreRestarts++
		s.logWarn("core exited; restarting…")
		// P1-13: reset stale turn/UI state so the restarted core isn't shown as
		// "working" with a dead request_id the user could accidentally approve.
		s.busy = false
		s.cur = nil
		s.queuedNext = false
		s.pendingApproval = nil
		s.pendingIntercom = nil
		s.subProgress = nil
		// clear stale per-core state so a crash-restart starts clean: a leftover
		// queued follow-up, pinned todos, a pending ask, footer metrics, or an
		// open settings/login modal would otherwise survive the restart and
		// reference the dead core's request ids / counts.
		s.queued = nil
		s.todos = nil
		s.pendingAsk = nil
		s.contextTokens = 0
		s.lastMetrics = nil
		s.lastCachePct = 0
		s.tokensSaved = 0
		s.umansConcUsed = nil
		s.umansConcLimit = nil
		if s.modal.kind != modalNone {
			s.closeModal()
		}
		// Stop the old stdin writer (its range loop exits on close) and drop the
		// dead pipes before respawning.
		if s.stdinCh != nil {
			close(s.stdinCh)
			s.stdinCh = nil
		}
		s.coreCmd = nil
		s.coreIn = nil
		return s, s.startCore()

	case coreEventMsg:
		return s, s.handleCoreEvent(msg.event)

	case tea.KeyPressMsg:
		model, cmd := s.handleKey(msg)
		return model, cmd

	case tea.MouseWheelMsg:
		return s, s.handleMouseWheel(msg)

	case tea.PasteMsg:
		// v2 enables bracketed-paste by default, so every paste arrives as a
		// PasteMsg (v1 delivered paste differently and this case was absent, which
		// silently dropped all pastes after the migration). Route the pasted text
		// to whichever input owns the keys right now — mirroring the key dispatch
		// in handleKey — so paste works in the chat box, the ask flyout, and modal
		// edit fields (e.g. pasting an API key). textinput v2 inserts the text.
		switch {
		case s.modal.kind != modalNone && s.modal.editing:
			s.modal.editBuf, _ = s.modal.editBuf.Update(msg)
		case s.pendingAsk != nil:
			q := &s.pendingAsk.questions[s.pendingAsk.focusIdx]
			q.input, _ = q.input.Update(msg)
		default:
			s.input, _ = s.input.Update(msg)
		}
		return s, nil

	default:
		// In v2, enhanced keyboard protocols (Kitty progressive keyboard + xterm
		// modifyOtherKeys) are auto-enabled by the renderer and every modified key
		// — Shift/Ctrl+Enter, Esc, Ctrl+letter — arrives as a real KeyPressMsg
		// dispatched by the case above. Nothing else to do here.
	}
	return s, nil
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

func main() {
	// CLI flags (--update / --check-update / --version / --help) are handled
	// before the TUI starts so they run in a plain terminal (no alt-screen) and
	// can exit cleanly.
	if code, handled := handleCLIArgs(os.Args[1:]); handled {
		os.Exit(code)
	}

	// v2 is declarative: alt-screen, mouse mode, and enhanced keyboard protocols
	// are set as fields on the View returned by View() rather than program options,
	// so NewProgram takes only the model. The renderer auto-enables the Kitty
	// progressive-keyboard + xterm modifyOtherKeys protocols (and restores the
	// terminal on exit), so the hand-rolled enable/disable sequences are gone.
	prog := tea.NewProgram(initialSession())

	// Background, non-blocking check for a newer release. On a fresh cache it
	// answers instantly (no network); otherwise it fetches asynchronously and
	// sends updateAvailableMsg when one is found. Silent on any failure.
	launchUpdateCheck(prog)

	// Kill the core child on SIGHUP (terminal closed) / SIGTERM (kill) so it
	// isn't orphaned and left running after the TUI exits. Best-effort: a missing
	// handle (core not yet started) just sends the quit msg. Instead of os.Exit
	// we send a sigtermMsg so Bubble Tea restores the terminal (alt-screen /
	// raw-mode) via its normal quit path — os.Exit would leave the terminal
	// broken. `quitting` is set first so the core's stdout EOF (from the kill)
	// doesn't trigger an auto-restart; the reader goroutine reaps the process.
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGTERM, syscall.SIGHUP)
	go func() {
		<-sigCh
		quitting.Store(true)
		if p := coreProcess.Load(); p != nil {
			_ = p.Kill()
		}
		prog.Send(sigtermMsg{})
	}()

	_, err := prog.Run()
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}
