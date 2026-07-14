package main

import (
	"strings"
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"
)

func TestWelcomeSurvivesStatusToast(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.authed = true
	s.layout()

	s.logInfo("2 model(s) discovered")
	s.logWarn("not authenticated — run /login")
	if s.hasConversation() {
		t.Fatal("status toasts must not count as conversation")
	}
	welcome := stripANSI(s.renderBlocks())
	if !strings.Contains(welcome, "Examples") {
		t.Fatalf("welcome should survive status toasts:\n%s", welcome)
	}
	if s.toast == nil {
		t.Fatal("logWarn should set a toast")
	}
}

func TestHistoryRecallDoesNotStompMultiline(t *testing.T) {
	s := initialSession()
	s.history = []string{"old prompt"}
	s.histIdx = 1
	s.input.SetValue("line1\nline2")
	s.input.SetCursor(len("line1\nli")) // mid second (last) line

	if s.historyRecallAllowed(-1) {
		t.Fatal("Up mid-draft must not recall history")
	}
	// On the last line, Down is allowed (shell/IDE: leave the draft downward).
	if !s.historyRecallAllowed(+1) {
		t.Fatal("Down on last line should allow history recall")
	}

	// Mid first line of a multi-line draft: Up allowed, Down blocked.
	s.input.SetValue("line1\nline2\nline3")
	s.input.SetCursor(2)
	if !s.historyRecallAllowed(-1) {
		t.Fatal("Up on first line should allow history recall")
	}
	if s.historyRecallAllowed(+1) {
		t.Fatal("Down on first line of multi-line draft must not recall")
	}

	s.input.SetCursor(len([]rune(s.input.Value())))
	if !s.historyRecallAllowed(+1) {
		t.Fatal("Down at end of last line should allow history recall")
	}
}

func TestQueueFullRefusesSecondFollowUp(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.authed = true
	s.busy = true
	s.models = []modelInfo{{ID: "m"}}
	s.queued = &queuedMsg{kind: "follow-up", text: "first", at: time.Now()}
	s.input.SetValue("second")

	_, _ = s.handleKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.input.Value() != "second" {
		t.Fatalf("queue-full refuse must keep draft, got %q", s.input.Value())
	}
	if s.queued == nil || s.queued.text != "first" {
		t.Fatal("queue-full refuse must keep the original queued prompt")
	}
}

func TestApprovalKeysIgnoredWhileTyping(t *testing.T) {
	s := initialSession()
	s.width, s.height = 80, 24
	s.pendingApproval = &approvalPrompt{requestID: "r1", tool: "bash", args: "{}"}
	s.input.SetValue("yes please")

	_, _ = s.handleKey(keyMsg("y"))
	if s.pendingApproval == nil {
		t.Fatal("y must not approve while composer has text")
	}
	s.input.Reset()
	_, _ = s.handleKey(keyMsg("y"))
	if s.pendingApproval != nil {
		t.Fatal("y should approve when composer is empty")
	}
}

func TestBareQuestionOpensHelp(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.authed = true
	s.layout()

	_, _ = s.handleKey(keyMsg("?"))
	if s.modal.kind != modalHelp {
		t.Fatalf("empty-input ? should open help, got kind=%v", s.modal.kind)
	}
}

func TestUnauthedWelcomeLeadsWithLogin(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.coreLifecycle = coreReady
	s.width, s.height = 80, 24
	s.authed = false
	s.layout()

	welcome := stripANSI(s.renderBlocks())
	if !strings.Contains(welcome, "Log in") && !strings.Contains(welcome, "/login") {
		t.Fatalf("unauthed welcome should lead with login:\n%s", welcome)
	}
}

func TestStartingWelcomeDoesNotClaimAPIKeyIsMissing(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.coreStartGen = 1
	s.width, s.height = 80, 24
	s.layout()

	welcome := stripANSI(s.renderBlocks())
	if strings.Contains(welcome, "No API key") || strings.Contains(welcome, "Log in first") {
		t.Fatalf("startup must not claim credentials are missing before ready:\n%s", welcome)
	}
	if !strings.Contains(welcome, "checking") || !strings.Contains(welcome, "credentials") {
		t.Fatalf("startup state missing credential check status:\n%s", welcome)
	}
}
