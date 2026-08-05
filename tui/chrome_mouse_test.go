package main

import (
	"strings"
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

// chromeSession builds a ready 90x36 session with a model selected and a
// writable settings path, mirroring mouseTranscriptSession without transcript
// blocks (chrome tests add content only when they need to scroll).
func chromeSession(t *testing.T) *session {
	t.Helper()
	s := initialSession()
	s.ready = true
	s.coreLifecycle = coreReady
	s.width, s.height = 90, 36
	s.models = []modelInfo{{ID: "test-model"}}
	s.modelIdx = 0
	s.settings.path = t.TempDir() + "/settings.json"
	s.layout()
	return s
}

// chromeClick performs a stationary press+release on a cell (the press/release
// pattern the transcript mouse layer uses for chrome targets).
func chromeClick(s *session, x, y int) tea.Cmd {
	s.handleTranscriptMouseClick(tea.MouseClickMsg{X: x, Y: y, Button: tea.MouseLeft})
	return s.handleTranscriptMouseRelease(tea.MouseReleaseMsg{X: x, Y: y, Button: tea.MouseLeft})
}

// chromeTextIndex locates a needle in a rendered chrome string, returning the
// (row, cell) of its first occurrence.
func chromeTextIndex(t *testing.T, text, needle string) (int, int) {
	t.Helper()
	lines := strings.Split(stripANSI(text), "\n")
	for i, ln := range lines {
		if j := strings.Index(ln, needle); j >= 0 {
			return i, lipgloss.Width(ln[:j])
		}
	}
	t.Fatalf("%q not found in:\n%s", needle, text)
	return 0, 0
}

// clickChromeText finds needle in the rendered chrome string and clicks the
// cell right after its first character (so the press lands inside the span).
// top is the screen row where the chrome zone begins: the string's row index
// is relative to the zone, not the screen.
func clickChromeText(t *testing.T, s *session, text, needle string, top int) tea.Cmd {
	t.Helper()
	row, x := chromeTextIndex(t, text, needle)
	return chromeClick(s, x+1, top+row)
}

func TestMouseCoordinateNormalizationIsExplicit(t *testing.T) {
	t.Setenv("CATCODE_MOUSE_Y_BIAS", "1")
	x, y := mouseCoord(7, 9)
	if x != 7 || y != 8 {
		t.Fatalf("one-based compatibility normalization = (%d,%d), want (7,8)", x, y)
	}
	t.Setenv("CATCODE_MOUSE_Y_BIAS", "")
	x, y = mouseCoord(7, 9)
	if x != 7 || y != 9 {
		t.Fatalf("normal Bubble Tea normalization = (%d,%d), want (7,9)", x, y)
	}
}

func TestChromeClickUsesPaintedScreenRow(t *testing.T) {
	s := chromeSession(t)
	s.input.SetValue("hello")
	s.input.Blur()
	s.layout()
	lay := s.chromeLayoutFor()
	if lay.inputTop <= 0 {
		t.Fatalf("invalid input top: %+v", lay)
	}
	// The first text row is the row visibly painted at inputTop+1. A click
	// there must focus the composer; the row above is the border and must not be
	// treated as text/cursor content.
	s.handleTranscriptMouseClick(tea.MouseClickMsg{X: 5, Y: lay.inputTop + 1, Button: tea.MouseLeft})
	if !s.input.Focused() {
		t.Fatal("painted composer row did not receive the click")
	}
	s.input.Blur()
	// Border cells are part of the composer hit target for focus (the visible
	// card is clickable as a whole), but cursor placement must not treat the
	// border as a text row.
	s.handleTranscriptMouseClick(tea.MouseClickMsg{X: 5, Y: lay.inputTop, Button: tea.MouseLeft})
	if !s.input.Focused() {
		t.Fatal("composer card border should still focus the input")
	}
	if pos := inputPosition(s.input); pos != len([]rune(s.input.Value())) {
		t.Fatalf("border click moved cursor to %d, want end %d", pos, len([]rune(s.input.Value())))
	}
}

func TestChromeMentionFlyoutClickSelectsItem(t *testing.T) {
	s := chromeSession(t)
	s.input.SetValue("@")
	s.mentionActive = true
	s.mentionAt = 0
	s.mentionItems = []mentionItem{
		{display: "main.go", insert: "main.go"},
		{display: "tui/", insert: "tui/", isDir: true},
		{display: "readme.md", insert: "readme.md"},
	}
	s.mentionCursor = 0
	s.mentionScroll = 0
	s.layout()

	lay := s.chromeLayoutFor()
	if lay.mentionRows == 0 {
		t.Fatal("mention flyout should be rendered")
	}
	// Row 1 of the flyout is the first item (below the top border).
	itemY := lay.mentionTop + 1
	s.handleTranscriptMouseClick(tea.MouseClickMsg{X: 3, Y: itemY, Button: tea.MouseLeft})
	if s.mentionCursor != 0 {
		t.Fatalf("press should highlight item 0, cursor=%d", s.mentionCursor)
	}
	cmd := s.handleTranscriptMouseRelease(tea.MouseReleaseMsg{X: 3, Y: itemY, Button: tea.MouseLeft})
	if cmd != nil {
		t.Fatal("mention accept should not issue a command")
	}
	if got := s.input.Value(); got != "@main.go " {
		t.Fatalf("input after click = %q, want %q", got, "@main.go ")
	}
	if s.mentionActive {
		t.Fatal("selecting a file should close the flyout")
	}
}

func TestChromeMentionFlyoutDragDoesNotAccept(t *testing.T) {
	s := chromeSession(t)
	s.input.SetValue("@")
	s.mentionActive = true
	s.mentionAt = 0
	s.mentionItems = []mentionItem{{display: "main.go", insert: "main.go"}}
	s.mentionCursor = 0
	s.layout()

	lay := s.chromeLayoutFor()
	itemY := lay.mentionTop + 1
	s.handleTranscriptMouseClick(tea.MouseClickMsg{X: 3, Y: itemY, Button: tea.MouseLeft})
	s.handleTranscriptMouseMotion(tea.MouseMotionMsg{X: 30, Y: itemY, Button: tea.MouseLeft})
	cmd := s.handleTranscriptMouseRelease(tea.MouseReleaseMsg{X: 30, Y: itemY, Button: tea.MouseLeft})
	if cmd != nil {
		t.Fatal("dragging on a mention row must not accept the item")
	}
	if s.input.Value() != "@" {
		t.Fatalf("drag changed the input to %q", s.input.Value())
	}
	if !s.mentionActive {
		t.Fatal("drag should not close the flyout")
	}
}

func TestChromeActivityShelfTogglesExpanded(t *testing.T) {
	s := chromeSession(t)
	s.subProgress = []*subProgressEntry{{agent: "worker", started: time.Now()}}
	s.layout()

	lay := s.chromeLayoutFor()
	if lay.shelfRows == 0 {
		t.Fatal("activity shelf should render")
	}
	if s.activityExpanded {
		t.Fatal("shelf should start collapsed")
	}
	clickChromeText(t, s, s.renderActivityShelf(), "Subagents 1 active", lay.shelfTop)
	if !s.activityExpanded {
		t.Fatal("collapsed shelf click should expand the shelf")
	}

	// The expanded panel's header row toggles back to collapsed.
	lay = s.chromeLayoutFor()
	headerY := lay.shelfTop + 1
	chromeClick(s, 2, headerY)
	if s.activityExpanded {
		t.Fatal("expanded shelf header click should collapse the shelf")
	}
}

func TestChromeGoalPanelClickTogglesCollapsed(t *testing.T) {
	s := chromeSession(t)
	s.goalState = &goalStateSnap{
		Phase:      "deploying",
		AutoDeploy: true,
		Prompts:    []goalPromptSnap{{Status: "done", Title: "write tests"}, {Status: "running", Title: "ship it"}},
	}
	s.layout()

	lay := s.chromeLayoutFor()
	if lay.goalRows == 0 {
		t.Fatal("goal panel should render")
	}
	if s.goalPanelCollapsed {
		t.Fatal("goal panel should start expanded")
	}
	chromeClick(s, 2, lay.goalTop+1)
	if !s.goalPanelCollapsed {
		t.Fatal("goal panel click should collapse the panel")
	}
	lay = s.chromeLayoutFor()
	chromeClick(s, 2, lay.goalTop+1)
	if s.goalPanelCollapsed {
		t.Fatal("collapsed goal panel click should re-expand")
	}
}

func TestChromePositionBarJumpsToBottom(t *testing.T) {
	s := chromeSession(t)
	for i := 0; i < 40; i++ {
		b := s.push(blkUser)
		b.appendText("scrollable transcript line")
	}
	s.cur = nil
	s.invalidateAll()
	s.refresh()
	s.viewport.ScrollUp(10)
	if s.renderPositionBar() == "" {
		t.Fatal("position bar should render after scrolling up")
	}
	lay := s.chromeLayoutFor()
	if lay.positionRows == 0 {
		t.Fatal("position bar row missing from chrome layout")
	}
	chromeClick(s, 2, lay.positionTop)
	if !s.viewport.AtBottom() {
		t.Fatalf("position bar click should jump to bottom, offset=%d", s.viewport.YOffset())
	}
	if !s.follow {
		t.Fatal("position bar click should re-pin follow mode")
	}
}

func TestChromeApprovalBannerZones(t *testing.T) {
	s := chromeSession(t)
	s.pendingApproval = &approvalPrompt{requestID: "r1", tool: "edit", args: `{"path":"a.go"}`, receivedAt: time.Now()}
	s.layout()

	// Approve once with an empty composer.
	lay := s.chromeLayoutFor()
	clickChromeText(t, s, s.renderApprovalBanner(), "[Y] once", lay.shelfTop)
	if s.pendingApproval != nil {
		t.Fatal("clicking the once zone should approve the request")
	}

	// Deny.
	s.pendingApproval = &approvalPrompt{requestID: "r2", tool: "edit", args: `{"path":"b.go"}`, receivedAt: time.Now()}
	s.layout()
	lay = s.chromeLayoutFor()
	clickChromeText(t, s, s.renderApprovalBanner(), "[N] deny", lay.shelfTop)
	if s.pendingApproval != nil {
		t.Fatal("clicking the deny zone should deny the request")
	}

	// Always allow.
	s.pendingApproval = &approvalPrompt{requestID: "r3", tool: "bash", args: `{"command":"make"}`, receivedAt: time.Now()}
	s.layout()
	lay = s.chromeLayoutFor()
	clickChromeText(t, s, s.renderApprovalBanner(), "[A] type", lay.shelfTop)
	if s.pendingApproval != nil {
		t.Fatal("clicking the always zone should allow permanently")
	}
}

func TestChromeApprovalZonesRespectEmptyComposer(t *testing.T) {
	s := chromeSession(t)
	s.pendingApproval = &approvalPrompt{requestID: "r1", tool: "edit", args: `{"path":"a.go"}`, receivedAt: time.Now()}
	s.input.SetValue("draft follow-up")
	s.layout()

	before := s.pendingApproval
	lay := s.chromeLayoutFor()
	clickChromeText(t, s, s.renderApprovalBanner(), "[Y] once", lay.shelfTop)
	if s.pendingApproval != before {
		t.Fatal("approval click must be ignored while the composer has a draft")
	}
	if s.toast == nil || s.toast.kind != toastWarn {
		t.Fatal("non-empty composer approval click should raise a warn toast")
	}
}

func TestChromeIntercomBannerSubmitAndSkip(t *testing.T) {
	s := chromeSession(t)
	s.pendingIntercom = &intercomPrompt{requestID: "i1", from: "worker", message: "proceed?"}
	s.input.SetValue("ok")
	s.layout()

	lay := s.chromeLayoutFor()
	clickChromeText(t, s, s.renderIntercomBanner(), "type reply + Enter", lay.shelfTop)
	if s.pendingIntercom != nil {
		t.Fatal("submit click should send the typed reply and advance the intercom")
	}

	// Skip sends the best-judgment nudge.
	s.pendingIntercom = &intercomPrompt{requestID: "i2", from: "worker", message: "proceed?"}
	s.layout()
	lay = s.chromeLayoutFor()
	clickChromeText(t, s, s.renderIntercomBanner(), "Esc skip", lay.shelfTop)
	if s.pendingIntercom != nil {
		t.Fatal("skip click should unblock the subagent with the no-reply nudge")
	}
}

func TestChromeQueueBannerCancelsQueue(t *testing.T) {
	s := chromeSession(t)
	s.busy = true
	s.queued = &queuedMsg{kind: "follow-up", text: "continue please", at: time.Now()}
	s.queuedNext = true
	s.layout()

	lay := s.chromeLayoutFor()
	clickChromeText(t, s, s.renderQueueBanner(), "queued follow-up", lay.shelfTop)
	if s.queued != nil || s.queuedNext {
		t.Fatalf("queue banner click should cancel the queue: queued=%v next=%v", s.queued, s.queuedNext)
	}
}

func TestChromeOauthURLSafety(t *testing.T) {
	for _, tc := range []struct {
		url  string
		safe bool
	}{
		{"https://example.com/oauth", true},
		{"http://localhost:8765/callback", true},
		{"http://127.0.0.1:8765/callback", true},
		{"http://[::1]:8765/callback", true},
		{"http://localhost.evil.example/oauth", false},
		{"http://127.0.0.1.evil.example/oauth", false},
		{"javascript:alert(1)", false},
	} {
		if got := isSafeOauthURL(tc.url); got != tc.safe {
			t.Errorf("isSafeOauthURL(%q) = %v, want %v", tc.url, got, tc.safe)
		}
	}
}

func TestChromeOauthBannerCopiesURL(t *testing.T) {
	s := chromeSession(t)
	s.oauth = &oauthBanner{message: "complete login", url: "https://example.com/auth?x=1"}
	s.layout()

	lay := s.chromeLayoutFor()
	cmd := clickChromeText(t, s, s.renderOauthBanner(), "complete login", lay.oauthTop)
	if cmd == nil {
		t.Fatal("OAuth banner click should issue clipboard commands")
	}
	if s.toast == nil || s.toast.kind != toastSuccess {
		t.Fatal("OAuth banner click should confirm the copy with a toast")
	}
}

func TestChromeBannerControlTextCannotCreateFalseHits(t *testing.T) {
	s := chromeSession(t)
	s.coreLifecycle = coreFailed
	s.coreFailure = "provider said q quit and r retry in its diagnostic"
	s.layout()
	lay := s.chromeLayoutFor()
	if hit := s.coreFailureHit(5, lay.failureTop, lay); hit.zone != chromeNone {
		t.Fatalf("diagnostic text must not be clickable: %+v", hit)
	}
	s.pendingIntercom = &intercomPrompt{requestID: "i1", from: "worker", message: "please do not esc skip this sentence"}
	s.coreLifecycle = coreReady
	s.layout()
	lay = s.chromeLayoutFor()
	if hit := s.intercomShelfHit(5, 0); hit.zone != chromeNone {
		t.Fatalf("intercom message must not be clickable: %+v", hit)
	}
}

func TestChromeCoreFailureBannerRetryAndQuit(t *testing.T) {
	s := chromeSession(t)
	s.coreLifecycle = coreFailed
	s.coreFailure = "boom"
	b := s.push(blkUser)
	b.appendText("recovery screen")
	s.cur = nil
	s.invalidateAll()
	s.refresh()
	s.layout()

	// Quit zone.
	lay := s.chromeLayoutFor()
	cmd := clickChromeText(t, s, s.renderCoreFailureBanner(), "q quit", lay.failureTop)
	if cmd == nil {
		t.Fatal("quit zone should issue the quit command")
	}
	if got := cmd(); got != (tea.QuitMsg{}) {
		t.Fatalf("quit zone command = %v, want tea.QuitMsg", got)
	}

	// Retry zone: point the core launcher at a nonexistent binary so the retry
	// restarts state without spawning a real subprocess.
	t.Setenv("CATCODE_CORE", "/nonexistent/catcode-core")
	s.coreLifecycle = coreFailed
	s.coreFailure = "boom"
	s.layout()
	lay = s.chromeLayoutFor()
	cmd = clickChromeText(t, s, s.renderCoreFailureBanner(), "r retry", lay.failureTop)
	if cmd == nil {
		t.Fatal("retry zone should issue a restart command")
	}
	if s.coreLifecycle != coreStarting {
		t.Fatalf("retry click should restart the core: lifecycle=%v", s.coreLifecycle)
	}
}

func TestChromeHeaderModelLabelOpensModelPicker(t *testing.T) {
	s := chromeSession(t)
	row, x := chromeTextIndex(t, s.renderHeader(), "· test-model")
	chromeClick(s, x+1, row)
	if s.modal.kind != modalModels {
		t.Fatalf("header model click should open the model picker, modal=%v", s.modal.kind)
	}
}

func TestChromeComposerClickFocusesAndPositionsCursor(t *testing.T) {
	s := chromeSession(t)
	s.input.SetValue("hello world")
	s.input.Blur()
	s.layout()

	lay := s.chromeLayoutFor()
	// Text starts at x=4 (border+padding+prefix); "hello " occupies cells 4-9,
	// so cell 10 is the 'w'. Clicking it puts the cursor before the 'w'.
	chromeClick(s, 10, lay.inputTop+1)
	if !s.input.Focused() {
		t.Fatal("composer click should focus the input")
	}
	if got := inputPosition(s.input); got != 6 {
		t.Fatalf("composer click cursor = %d, want 6 (before \"world\")", got)
	}
}

func TestChromeFooterControls(t *testing.T) {
	s := chromeSession(t)

	// Commands opens the palette.
	lay := s.chromeLayoutFor()
	clickChromeText(t, s, s.renderFooter(), "commands", lay.footerTop)
	if s.modal.kind != modalCommand {
		t.Fatalf("footer commands click should open the command palette, modal=%v", s.modal.kind)
	}
	s.closeModal()

	// Send submits the composer.
	s.authed = true
	s.input.SetValue("hello from footer")
	lay = s.chromeLayoutFor()
	clickChromeText(t, s, s.renderFooter(), "send", lay.footerTop)
	if !s.busy {
		t.Fatal("footer send click should start a turn")
	}

	// While busy, the abort control peels the queue first (Esc semantics).
	s.busy = true
	s.queued = &queuedMsg{kind: "follow-up", text: "queued", at: time.Now()}
	s.queuedNext = true
	s.layout()
	lay = s.chromeLayoutFor()
	clickChromeText(t, s, s.renderFooter(), "abort", lay.footerTop)
	if s.queued != nil || s.queuedNext {
		t.Fatalf("footer abort click should cancel the queue: queued=%v next=%v", s.queued, s.queuedNext)
	}
}

func TestChromeFooterToastIsNotClickable(t *testing.T) {
	s := chromeSession(t)
	s.setToast(toastInfo, "steer cancelled")
	lay := s.chromeLayoutFor()
	if hit := s.chromeHitAt(8, lay.footerTop); hit.zone != chromeNone {
		t.Fatalf("toast must not expose footer action: %+v", hit)
	}
}

func TestChromeFooterApprovalGuard(t *testing.T) {
	s := chromeSession(t)
	s.pendingApproval = &approvalPrompt{requestID: "r1", tool: "edit", args: `{"path":"a.go"}`, receivedAt: time.Now()}
	s.input.SetValue("draft")
	s.layout()

	before := s.pendingApproval
	lay := s.chromeLayoutFor()
	clickChromeText(t, s, s.renderFooter(), "always allow type", lay.footerTop)
	if s.pendingApproval != before {
		t.Fatal("footer approval click must respect the empty-composer guard")
	}
}

func TestChromePressCancelledWhenAskArrives(t *testing.T) {
	s := chromeSession(t)
	lay := s.chromeLayoutFor()
	s.handleTranscriptMouseClick(tea.MouseClickMsg{X: 2, Y: lay.footerTop, Button: tea.MouseLeft})
	s.pendingAsk = &askPrompt{requestID: "a1"}
	if cmd := s.handleTranscriptMouseRelease(tea.MouseReleaseMsg{X: 2, Y: lay.footerTop, Button: tea.MouseLeft}); cmd != nil || s.chromePress.active {
		t.Fatal("a blocking ask overlay must cancel the old chrome press")
	}
}

func TestChromeOverlayPrecedence(t *testing.T) {
	// Blocking ask flyout swallows chrome clicks entirely.
	s := chromeSession(t)
	s.pendingAsk = &askPrompt{requestID: "a1"}
	s.layout()
	lay := s.chromeLayoutFor()
	s.viewport.ScrollUp(3)
	offsetBefore := s.viewport.YOffset()
	cmd := chromeClick(s, 2, lay.positionTop)
	if cmd != nil || s.viewport.YOffset() != offsetBefore {
		t.Fatal("chrome click must be swallowed while the ask flyout is open")
	}

	// A modal owns all clicks (existing behavior preserved).
	s2 := chromeSession(t)
	s2.openHelp()
	s2.layout()
	_ = s2.View()
	y, x := chromeTextIndex(t, s2.renderModalOverlay("base"), "Help")
	s2.handleTranscriptMouseClick(tea.MouseClickMsg{X: x + 1, Y: y, Button: tea.MouseLeft})
	if !s2.modalSelection.active {
		t.Fatal("click with a modal open should start a modal selection")
	}
}

func TestMouseWheelSwallowedByAskAndSudo(t *testing.T) {
	for _, overlay := range []string{"ask", "sudo"} {
		s := chromeSession(t)
		for i := 0; i < 40; i++ {
			b := s.push(blkUser)
			b.appendText("line")
		}
		s.cur = nil
		s.invalidateAll()
		s.refresh()
		if overlay == "ask" {
			s.pendingAsk = &askPrompt{requestID: "a1"}
		} else {
			s.pendingSudo = &sudoPrompt{requestID: "s1"}
		}
		s.layout()
		s.viewport.ScrollUp(5)
		offsetBefore := s.viewport.YOffset()
		cmd := s.handleMouseWheel(tea.MouseWheelMsg{Button: tea.MouseWheelUp})
		if cmd != nil {
			t.Fatalf("%s overlay: wheel should be swallowed, got cmd", overlay)
		}
		if got := s.viewport.YOffset(); got != offsetBefore {
			t.Fatalf("%s overlay: wheel scrolled hidden transcript: before=%d after=%d", overlay, offsetBefore, got)
		}
	}
}
