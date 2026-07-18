package main

import (
	"encoding/json"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

// Bare slash commands that used to print "usage: …" must open modals instead.
// Skills remain argument-oriented (see command_palette_test.go).

func TestBareArgCommandsOpenModals(t *testing.T) {
	cases := []struct {
		cmd    string
		kind   modalKind
		target string // for modalValueEdit
	}{
		{"/steer", modalValueEdit, editTargetSteer},
		{"/attach", modalAttachFile, ""},
		{"/remember", modalValueEdit, editTargetRemember},
		{"/plugin-install", modalValueEdit, editTargetPluginInstall},
		{"/run", modalValueEdit, editTargetRun},
		{"/parallel", modalValueEdit, editTargetParallel},
		{"/chain", modalValueEdit, editTargetChain},
		{"/compact", modalValueEdit, editTargetCompact},
		{"/oauth-code", modalOauthCode, ""},
		{"/approval", modalApproval, ""},
		{"/reasoning", modalReasoning, ""},
		{"/theme", modalTheme, ""},
		{"/model", modalModels, ""},
		{"/settings", modalSettings, ""},
		{"/goal", modalGoal, ""},
	}
	for _, tc := range cases {
		t.Run(tc.cmd, func(t *testing.T) {
			s := initialSession()
			s.ready = true
			s.width, s.height = 80, 24
			s.handleUserLine(tc.cmd)
			if s.modal.kind != tc.kind {
				t.Fatalf("%s: kind=%v, want %v", tc.cmd, s.modal.kind, tc.kind)
			}
			if tc.target != "" && s.modal.editTarget != tc.target {
				t.Fatalf("%s: editTarget=%q, want %q", tc.cmd, s.modal.editTarget, tc.target)
			}
			if tc.kind == modalValueEdit && !s.modal.editing {
				t.Fatalf("%s: value-edit modal should be editing", tc.cmd)
			}
		})
	}
}

func TestBareForgetAndMemoryRequestPicker(t *testing.T) {
	for _, cmd := range []string{"/forget", "/memory"} {
		s := initialSession()
		s.ready = true
		s.handleUserLine(cmd)
		if !s.pendingMemoryPicker {
			t.Errorf("%s: expected pendingMemoryPicker", cmd)
		}
		if s.modal.kind != modalMemory || !s.modal.loading {
			t.Errorf("%s: should open a durable loading modal; kind=%v loading=%v", cmd, s.modal.kind, s.modal.loading)
		}
	}
}

func TestMemoryListEventOpensPicker(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.pendingMemoryPicker = true
	raw, _ := json.Marshal(map[string]any{
		"type": "memory_list",
		"entries": []map[string]any{
			{"id": "m1", "text": "use modals for slash args", "tags": []string{"ux"}},
		},
	})
	s.handleCoreEvent(&coreEvent{Type: "memory_list", Raw: raw})
	if s.modal.kind != modalMemory {
		t.Fatalf("kind=%v, want modalMemory", s.modal.kind)
	}
	if s.pendingMemoryPicker {
		t.Fatal("pendingMemoryPicker should clear after open")
	}
	if len(s.memoryList) != 1 || s.memoryList[0].ID != "m1" {
		t.Fatalf("memoryList=%v", s.memoryList)
	}
}

func TestMemoryPickerEnterForgets(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.memoryList = []memoryEntry{{ID: "abc", Text: "note one"}, {ID: "def", Text: "note two"}}
	s.openMemoryPicker()
	if s.modal.kind != modalMemory {
		t.Fatalf("kind=%v", s.modal.kind)
	}
	// Select first row.
	s.modal.cursor = 0
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.modal.kind != modalConfirm || len(s.memoryList) != 2 {
		t.Fatalf("forget must require confirmation; kind=%v list=%v", s.modal.kind, s.memoryList)
	}
	// huh Confirm: Cancel is default; 'y' accepts (Confirm).
	s.handleModalKey(tea.KeyPressMsg{Code: 'y', Text: "y"})
	if len(s.memoryList) != 1 || s.memoryList[0].ID != "def" {
		t.Fatalf("after forget: memoryList=%v", s.memoryList)
	}
	if s.modal.kind != modalNone {
		t.Fatalf("confirmation should close after action; kind=%v", s.modal.kind)
	}
}

func TestBarePluginCommandsRequestPicker(t *testing.T) {
	cases := []struct {
		cmd  string
		mode string
	}{
		{"/plugin-config", pluginModeToggle},
		{"/plugin-list", pluginModeToggle},
		{"/plugin-enable", pluginModeToggle},
		{"/plugin-disable", pluginModeToggle},
		{"/plugin-remove", pluginModeRemove},
	}
	for _, tc := range cases {
		s := initialSession()
		s.ready = true
		s.handleUserLine(tc.cmd)
		if s.pluginPickerMode != tc.mode {
			t.Errorf("%s: pluginPickerMode=%q, want %q", tc.cmd, s.pluginPickerMode, tc.mode)
		}
	}
}

func TestRememberModalCommitSaves(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.handleUserLine("/remember")
	if s.modal.kind != modalValueEdit || s.modal.editTarget != editTargetRemember {
		t.Fatalf("expected remember modal, got kind=%v target=%q", s.modal.kind, s.modal.editTarget)
	}
	s.modal.editBuf.SetValue("remember to open modals")
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.modal.kind != modalNone {
		t.Fatalf("modal should close after commit; kind=%v", s.modal.kind)
	}
}

func TestSteerModalCommitSends(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.authed = true
	s.models = []modelInfo{{ID: "m1"}}
	s.modelIdx = 0
	s.busy = true // steer is typically mid-turn
	s.handleUserLine("/steer")
	s.modal.editBuf.SetValue("focus on the failing test")
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.modal.kind != modalNone {
		t.Fatalf("modal should close; kind=%v", s.modal.kind)
	}
	if s.queued == nil || s.queued.kind != "steer" {
		t.Fatalf("expected queued steer, got %+v", s.queued)
	}
}

func TestRunModalCommitDelegates(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.authed = true
	s.models = []modelInfo{{ID: "m1"}}
	s.modelIdx = 0
	s.handleUserLine("/run")
	if s.modal.editTarget != editTargetRun {
		t.Fatalf("editTarget=%q", s.modal.editTarget)
	}
	s.modal.editBuf.SetValue(`scout "map the auth package"`)
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.modal.kind != modalNone {
		t.Fatalf("modal should close; kind=%v", s.modal.kind)
	}
	if !s.busy {
		t.Fatal("run should dispatch a turn (busy)")
	}
}

func TestGoalModalPrefillAndSubmit(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.authed = true
	s.models = []modelInfo{{ID: "m1", Provider: "openai"}}
	s.modelIdx = 0
	s.settings = &settingsStore{ReasoningEffort: "medium"}
	s.handleUserLine("/goal fix the auth flow")
	if s.modal.kind != modalGoal {
		t.Fatalf("kind=%v, want modalGoal", s.modal.kind)
	}
	if s.goalDraft.goal != "fix the auth flow" {
		t.Fatalf("prefill goal=%q", s.goalDraft.goal)
	}
	// Jump to Start and submit.
	s.goalDraft.field = goalFieldStart
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.modal.kind != modalNone {
		t.Fatalf("modal should close after submit; kind=%v", s.modal.kind)
	}
	if !s.busy {
		t.Fatal("start_goal should mark session busy")
	}
}

func TestGoalModalProfileAndLayout(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 40
	s.models = []modelInfo{{ID: "m1", Provider: "openai"}, {ID: "m2", Provider: "openai"}}
	s.providers = []string{"openai"}
	s.openGoalModal("ship the release")

	body := stripANSI(s.renderGoalModal())
	for _, want := range []string{"Goal Mode", "Run", "Deploy", "Scope", "Profile", "After plan", "Start goal", "auto-deploy"} {
		if !strings.Contains(body, want) {
			t.Fatalf("missing %q in goal modal:\n%s", want, body)
		}
	}

	s.goalDraft.field = goalFieldProfile
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyRight})
	if s.goalDraft.concurrency != 8 {
		t.Fatalf("Ultra profile concurrency=%d, want 8", s.goalDraft.concurrency)
	}
	if s.goalDraft.maxTasks < 16 {
		t.Fatalf("Ultra profile should raise max tasks, got %d", s.goalDraft.maxTasks)
	}

	s.goalDraft.field = goalFieldReview
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyRight})
	if !s.goalDraft.reviewBeforeDeploy {
		t.Fatal("right on After plan should select Review first")
	}
	body = stripANSI(s.renderGoalModal())
	if !strings.Contains(body, "review plan first") {
		t.Fatalf("summary should reflect review mode:\n%s", body)
	}
}

func TestGoalStatePlanReadyOpensReview(t *testing.T) {
	s := initialSession()
	s.ready = true
	raw, _ := json.Marshal(map[string]any{
		"type":        "goal_state",
		"id":          "goal-1",
		"goal":        "ship feature",
		"phase":       "plan_ready",
		"auto_deploy": false,
		"version":     2,
		"prompts": []map[string]any{
			{"step_id": "1", "agent": "worker", "title": "impl", "status": "pending"},
		},
	})
	s.handleCoreEvent(&coreEvent{Type: "goal_state", Raw: raw})
	if s.modal.kind != modalGoalPlan {
		t.Fatalf("kind=%v, want modalGoalPlan", s.modal.kind)
	}
	if s.goalState == nil || len(s.goalState.Prompts) != 1 {
		t.Fatalf("goalState=%+v", s.goalState)
	}
}

func TestSkillStillRequiresNameNotModal(t *testing.T) {
	s := initialSession()
	s.ready = true
	// Incomplete skill token is still a usage/error path, not a modal.
	s.handleUserLine("/skill:")
	if s.modal.kind != modalNone {
		t.Fatalf("skills must not open a value modal; kind=%v", s.modal.kind)
	}
}

func TestCommandsWithArgsStillApplyDirectly(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.authed = true
	s.models = []modelInfo{{ID: "m1"}}
	s.modelIdx = 0
	// Direct args must not open modals (power-user / scripting path).
	s.handleUserLine("/remember keep the modal rule")
	if s.modal.kind != modalNone {
		t.Fatalf("/remember with args opened modal kind=%v", s.modal.kind)
	}
	s.handleUserLine("/approval always")
	if s.modal.kind != modalNone {
		t.Fatalf("/approval with args opened modal kind=%v", s.modal.kind)
	}
	if s.settings.Approval != "always" {
		t.Fatalf("approval=%q, want always", s.settings.Approval)
	}
}

func TestPluginRemoveSelectUninstalls(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.pluginPickerMode = pluginModeRemove
	raw, _ := json.Marshal(map[string]any{
		"name": "demo", "version": "1.0", "description": "d", "enabled": true,
	})
	s.openPluginPicker([]json.RawMessage{raw})
	if s.modal.kind != modalPlugins {
		t.Fatalf("kind=%v", s.modal.kind)
	}
	s.handleModalKey(tea.KeyPressMsg{Code: tea.KeyEnter})
	if s.modal.kind != modalConfirm || len(sPluginStore) != 1 {
		t.Fatalf("uninstall must require confirmation; kind=%v len=%d", s.modal.kind, len(sPluginStore))
	}
	s.handleModalKey(tea.KeyPressMsg{Code: 'y', Text: "y"})
	if len(sPluginStore) != 0 {
		t.Fatalf("plugin should be dropped from store; len=%d", len(sPluginStore))
	}
}

func TestPaletteSkillStillInsertsNotModal(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.width, s.height = 80, 24
	s.skillsList = []skillInfo{{Name: "review", Description: "code review"}}
	s.openCommandPalette()
	idx := -1
	for i, it := range s.commandItems() {
		if it.label == "/skill:review" {
			idx = i
			break
		}
	}
	if idx < 0 {
		t.Fatal("skill missing from palette")
	}
	s.runCommandByIndex(idx)
	if got := s.input.Value(); got != "/skill:review " {
		t.Fatalf("skill should insert into input, got %q", got)
	}
	if s.modal.kind != modalNone {
		t.Fatalf("palette should close after skill insert; kind=%v", s.modal.kind)
	}
}
