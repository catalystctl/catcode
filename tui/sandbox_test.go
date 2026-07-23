package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

// captureWriter is a minimal io.WriteCloser that records the JSONL commands the
// TUI would send to the core's stdin. In tests s.stdinCh is nil, so sendCore
// writes synchronously through coreIn.Write — no goroutine or timing needed.
type captureWriter struct {
	lines []string
}

func (c *captureWriter) Write(b []byte) (int, error) {
	c.lines = append(c.lines, strings.TrimRight(string(b), "\n"))
	return len(b), nil
}

func (c *captureWriter) Close() error { return nil }

// sentType returns the "type" of the n-th captured command (0-indexed).
func (c *captureWriter) sentType(n int) string {
	if n < 0 || n >= len(c.lines) {
		return ""
	}
	var m map[string]any
	if json.Unmarshal([]byte(c.lines[n]), &m) != nil {
		return ""
	}
	t, _ := m["type"].(string)
	return t
}

func newSandboxSession(t *testing.T) *session {
	t.Helper()
	s := initialSession()
	s.ready = true
	s.settings.path = filepath.Join(t.TempDir(), "settings.json")
	s.coreIn = &captureWriter{}
	return s
}

// TestReadyEventParsesSandboxFields: the ready event must populate the runtime
// snapshot (effective mode, readiness, image, cpu/mem, network) so the status
// panel reflects what the core actually resolved.
func TestReadyEventParsesSandboxFields(t *testing.T) {
	s := newSandboxSession(t)
	s.handleCoreEvent(rawEvent(t, "ready", map[string]any{
		"sandbox":            "microsandbox",
		"sandboxReady":       true,
		"sandboxImage":       "ghcr.io/catalystctl/catcode-sandbox:0.2.0",
		"sandboxCpus":        2,
		"sandboxMemoryMb":    2048,
		"sandboxNetworkMode": "restricted",
		"models":             []any{},
	}))
	snap := s.sandboxStatus
	if snap == nil {
		t.Fatal("sandboxStatus snapshot not populated by ready event")
	}
	if snap.Mode != "microsandbox" || !snap.Ready {
		t.Errorf("snapshot = %+v, want mode=microsandbox ready=true", snap)
	}
	if snap.Image != "ghcr.io/catalystctl/catcode-sandbox:0.2.0" {
		t.Errorf("image = %q", snap.Image)
	}
	if snap.Cpus != 2 || snap.MemoryMb != 2048 {
		t.Errorf("cpus=%d mem=%d, want 2/2048", snap.Cpus, snap.MemoryMb)
	}
	if snap.NetworkMode != "restricted" {
		t.Errorf("network = %q, want restricted", snap.NetworkMode)
	}
}

// TestSandboxStatusEventPopulatesReport: sandbox_status carries the full
// preflight report (checks + actions), which the panel renders.
func TestSandboxStatusEventPopulatesReport(t *testing.T) {
	s := newSandboxSession(t)
	s.handleCoreEvent(rawEvent(t, "sandbox_status", map[string]any{
		"mode": "microsandbox",
		"report": map[string]any{
			"ready":        false,
			"supported":    true,
			"platform":     "linux",
			"architecture": "x86_64",
			"checks": []any{
				map[string]any{"code": "kvm_device_missing", "title": "KVM device", "status": "fail", "detail": "/dev/kvm not found"},
			},
			"actions": []any{
				map[string]any{"title": "Enable KVM", "explanation": "Enable VT-x/AMD-V in BIOS", "command": "sudo modprobe kvm", "requires_admin": true, "requires_reboot": false},
			},
		},
	}))
	snap := s.sandboxStatus
	if snap == nil || !snap.HasReport || snap.Report == nil {
		t.Fatalf("report not populated: %+v", snap)
	}
	if snap.Ready {
		t.Error("ready should be false")
	}
	if !snap.Supported {
		t.Error("supported should be true")
	}
	if snap.Platform != "linux" || snap.Architecture != "x86_64" {
		t.Errorf("platform/arch = %q/%q", snap.Platform, snap.Architecture)
	}
	if len(snap.Report.Checks) != 1 || snap.Report.Checks[0].Code != "kvm_device_missing" {
		t.Errorf("checks = %#v", snap.Report.Checks)
	}
	if len(snap.Report.Actions) != 1 || snap.Report.Actions[0].Command != "sudo modprobe kvm" {
		t.Errorf("actions = %#v", snap.Report.Actions)
	}
}

// TestSandboxEnableReadyPersists: when the user enables Microsandbox and the
// core reports the environment ready, the setting is persisted + a restart is
// offered. Fail-closed: nothing is persisted until ready.
func TestSandboxEnableReadyPersists(t *testing.T) {
	s := newSandboxSession(t)
	s.handleUserLine("/sandbox enable")
	if !s.pendingSandboxEnable {
		t.Fatal("enable should set pendingSandboxEnable")
	}
	if s.modal.kind != modalSandboxStatus {
		t.Errorf("modal = %v, want modalSandboxStatus", s.modal.kind)
	}
	if got := s.coreIn.(*captureWriter).sentType(0); got != "get_sandbox_status" {
		t.Errorf("first command = %q, want get_sandbox_status", got)
	}
	// Core replies: environment ready.
	s.handleCoreEvent(rawEvent(t, "sandbox_status", map[string]any{
		"mode":   "microsandbox",
		"report": map[string]any{"ready": true, "supported": true, "platform": "linux"},
	}))
	if s.pendingSandboxEnable {
		t.Error("pendingSandboxEnable should clear after ready reply")
	}
	if s.settings.Sandbox != "microsandbox" {
		t.Errorf("settings.Sandbox = %q, want microsandbox", s.settings.Sandbox)
	}
	if s.modal.kind != modalConfirm {
		t.Errorf("modal = %v, want modalConfirm (restart hint)", s.modal.kind)
	}
}

// TestSandboxEnableNotReadyDoesNotPersist: a not-ready environment must never
// silently save "none" (or "microsandbox") on the user's behalf — the status
// panel stays open with setup guidance.
func TestSandboxEnableNotReadyDoesNotPersist(t *testing.T) {
	s := newSandboxSession(t)
	s.handleUserLine("/sandbox enable")
	s.handleCoreEvent(rawEvent(t, "sandbox_status", map[string]any{
		"mode":   "microsandbox",
		"report": map[string]any{"ready": false, "supported": true, "platform": "linux", "actions": []any{}},
	}))
	if s.settings.Sandbox != "none" {
		t.Errorf("settings.Sandbox = %q, want none (must not persist until ready)", s.settings.Sandbox)
	}
	if s.modal.kind != modalSandboxStatus {
		t.Errorf("modal = %v, want modalSandboxStatus (guidance stays open)", s.modal.kind)
	}
}

// TestSandboxErrorCancelsPendingEnable: a sandbox_error cancels the pending
// enable so the user must explicitly re-request it after fixing setup.
func TestSandboxErrorCancelsPendingEnable(t *testing.T) {
	s := newSandboxSession(t)
	s.handleUserLine("/sandbox enable")
	s.handleCoreEvent(rawEvent(t, "sandbox_error", map[string]any{
		"error": "image_pull_failed",
	}))
	if s.pendingSandboxEnable {
		t.Error("pendingSandboxEnable should clear on sandbox_error")
	}
	if s.sandboxStatus == nil || s.sandboxStatus.Error != "image_pull_failed" {
		t.Errorf("error not recorded: %+v", s.sandboxStatus)
	}
	if s.settings.Sandbox != "none" {
		t.Errorf("settings.Sandbox = %q, want none (fail-closed)", s.settings.Sandbox)
	}
}

// TestSandboxPrepareProgressThenReady: prepare-progress sets the phase; the
// sandbox_ready event clears it and marks readiness.
func TestSandboxPrepareProgressThenReady(t *testing.T) {
	s := newSandboxSession(t)
	s.openSandboxStatusModal()
	s.setSandboxModalLoading(true)
	s.handleCoreEvent(rawEvent(t, "sandbox_prepare_progress", map[string]any{
		"phase": "downloading runtime",
	}))
	if s.sandboxStatus.PreparePhase != "downloading runtime" {
		t.Errorf("phase = %q", s.sandboxStatus.PreparePhase)
	}
	s.handleCoreEvent(rawEvent(t, "sandbox_ready", map[string]any{
		"ready":  true,
		"report": map[string]any{"ready": true, "supported": true},
	}))
	if s.sandboxStatus.PreparePhase != "" {
		t.Errorf("phase should clear on ready, got %q", s.sandboxStatus.PreparePhase)
	}
	if !s.sandboxStatus.Ready {
		t.Error("ready should be true after sandbox_ready")
	}
}

// TestSandboxSubcommandsDispatch: the /sandbox subcommands issue the right
// protocol commands and drive the expected UI state.
func TestSandboxSubcommandsDispatch(t *testing.T) {
	cases := []struct {
		cmd       string
		wantType  string // "" = no core command expected (e.g. disable is local)
		wantModal modalKind
	}{
		{"/sandbox status", "get_sandbox_status", modalSandboxStatus},
		{"/sandbox enable", "get_sandbox_status", modalSandboxStatus},
		{"/sandbox recheck", "get_sandbox_status", modalSandboxStatus},
		{"/sandbox setup", "prepare_sandbox", modalSandboxStatus},
		{"/sandbox reset", "reset_sandbox", modalSandboxStatus},
		{"/sandbox disable", "", modalConfirm},
	}
	for _, tc := range cases {
		t.Run(tc.cmd, func(t *testing.T) {
			s := newSandboxSession(t)
			s.handleUserLine(tc.cmd)
			cw := s.coreIn.(*captureWriter)
			if tc.wantType != "" {
				if got := cw.sentType(0); got != tc.wantType {
					t.Errorf("command = %q, want %q", got, tc.wantType)
				}
			} else if len(cw.lines) != 0 {
				t.Errorf("expected no core command, got %v", cw.lines)
			}
			if s.modal.kind != tc.wantModal {
				t.Errorf("modal = %v, want %v", s.modal.kind, tc.wantModal)
			}
			if tc.cmd == "/sandbox disable" && s.settings.Sandbox != "none" {
				t.Errorf("settings.Sandbox = %q, want none", s.settings.Sandbox)
			}
		})
	}
}

// TestSandboxValueArgsMigrate: /sandbox <deprecated> preserves the intent to
// sandbox (migrates to microsandbox + enable flow), while /sandbox microsandbox
// also enables. Unknown values are rejected.
func TestSandboxValueArgsMigrate(t *testing.T) {
	for _, arg := range []string{"firejail", "fj", "seatbelt", "macos", "sandbox-exec"} {
		t.Run(arg, func(t *testing.T) {
			s := newSandboxSession(t)
			s.handleUserLine("/sandbox " + arg)
			if !s.pendingSandboxEnable {
				t.Errorf("/sandbox %s should migrate to enable", arg)
			}
			if s.settings.Sandbox != "none" {
				t.Errorf("settings.Sandbox = %q, want none (not persisted until ready)", s.settings.Sandbox)
			}
		})
	}
	// Unknown value is rejected, not silently coerced.
	s := newSandboxSession(t)
	s.handleUserLine("/sandbox bogus")
	if s.pendingSandboxEnable {
		t.Error("unknown sandbox value should not enable")
	}
}

// TestSandboxSettingsMigrationOnLoad: a persisted deprecated backend is
// migrated to microsandbox on load (intent preserved, never none), and the
// original value is surfaced for a one-time deprecation notice.
func TestSandboxSettingsMigrationOnLoad(t *testing.T) {
	cases := []struct {
		disk     string
		wantMode string
		migrated string
	}{
		{`{"sandbox":"firejail"}`, "microsandbox", "firejail"},
		{`{"sandbox":"seatbelt"}`, "microsandbox", "seatbelt"},
		{`{"sandbox":"macos"}`, "microsandbox", "macos"},
		{`{"sandbox":"microsandbox"}`, "microsandbox", ""},
		{`{"sandbox":"none"}`, "none", ""},
	}
	for _, tc := range cases {
		t.Run(tc.disk, func(t *testing.T) {
			dir := t.TempDir()
			path := filepath.Join(dir, "settings.json")
			if err := os.WriteFile(path, []byte(tc.disk), 0600); err != nil {
				t.Fatal(err)
			}
			st := loadSettingsFrom(path)
			if st.Sandbox != tc.wantMode {
				t.Errorf("Sandbox = %q, want %q", st.Sandbox, tc.wantMode)
			}
			if st.migratedSandbox != tc.migrated {
				t.Errorf("migratedSandbox = %q, want %q", st.migratedSandbox, tc.migrated)
			}
		})
	}
}

// TestSandboxStatusModalRenders: the status panel surfaces the summary
// (platform/image/limits/network), the failing check, and the copyable setup
// command with its admin badge.
func TestSandboxStatusModalRenders(t *testing.T) {
	s := newSandboxSession(t)
	s.width, s.height = 100, 40
	s.openSandboxStatusModal()
	s.handleCoreEvent(rawEvent(t, "sandbox_status", map[string]any{
		"mode": "microsandbox",
		"report": map[string]any{
			"ready":        false,
			"supported":    true,
			"platform":     "linux",
			"architecture": "x86_64",
			"checks": []any{
				map[string]any{"code": "kvm_permission_denied", "title": "KVM access", "status": "fail", "detail": "no read/write on /dev/kvm"},
			},
			"actions": []any{
				map[string]any{"title": "Grant KVM access", "explanation": "Add your user to the kvm group", "command": "sudo usermod -aG kvm \"$USER\"", "requires_admin": true, "requires_reboot": false},
			},
		},
	}))
	body := stripANSI(s.renderModalBody())
	for _, want := range []string{"Microsandbox", "linux", "x86_64", "not ready", "KVM access", "Grant KVM access", "sudo usermod -aG kvm", "requires admin"} {
		if !strings.Contains(body, want) {
			t.Errorf("status modal missing %q:\n%s", want, body)
		}
	}
}

// TestSandboxStatusModalKeyDispatch: pressing p/r inside the status modal
// routes through handleSandboxStatusKey and issues the right protocol command.
func TestSandboxStatusModalKeyDispatch(t *testing.T) {
	cases := []struct {
		key      string
		keyrune  rune
		wantType string
	}{
		{"r", 'r', "get_sandbox_status"},
		{"p", 'p', "prepare_sandbox"},
	}
	for _, tc := range cases {
		t.Run(tc.key, func(t *testing.T) {
			s := newSandboxSession(t)
			s.openSandboxStatusModal()
			// Seed a report so the modal is not in its initial loading state.
			s.handleCoreEvent(rawEvent(t, "sandbox_status", map[string]any{
				"mode":   "microsandbox",
				"report": map[string]any{"ready": false, "supported": true, "platform": "linux", "actions": []any{}},
			}))
			cw := s.coreIn.(*captureWriter)
			cw.lines = cw.lines[:0] // ignore the openSandboxStatusModal get_sandbox_status
			s.handleModalKey(tea.KeyPressMsg{Code: tc.keyrune})
			if got := cw.sentType(0); got != tc.wantType {
				t.Errorf("key %q sent %q, want %q", tc.key, got, tc.wantType)
			}
		})
	}
}

// TestSandboxCopyCommand copies the first setup command to the clipboard
// (best-effort: the clipboard may be unavailable in CI, so we only assert no
// crash and a toast is set).
func TestSandboxCopyCommandNoCrash(t *testing.T) {
	s := newSandboxSession(t)
	s.openSandboxStatusModal()
	s.handleCoreEvent(rawEvent(t, "sandbox_status", map[string]any{
		"mode": "microsandbox",
		"report": map[string]any{
			"ready": false, "supported": true, "platform": "linux",
			"actions": []any{
				map[string]any{"title": "Grant KVM", "command": "sudo usermod -aG kvm \"$USER\"", "requires_admin": true},
			},
		},
	}))
	s.handleModalKey(tea.KeyPressMsg{Code: 'c'})
	if s.toast == nil {
		t.Error("copy command should set a toast (success or unavailable notice)")
	}
}

// TestSandboxStatusCommandReportsEffectiveState: /status must surface the
// effective (core-reported) sandbox state so the TUI/web/CLI/model prompt
// agree (acceptance criterion 15).
func TestSandboxStatusCommandReportsEffectiveState(t *testing.T) {
	s := newSandboxSession(t)
	s.handleCoreEvent(rawEvent(t, "ready", map[string]any{
		"sandbox":            "microsandbox",
		"sandboxReady":       true,
		"sandboxNetworkMode": "restricted",
		"models":             []any{},
	}))
	s.handleUserLine("/status")
	// /status persists to the transcript via logPersist; inspect the last block.
	if len(s.blocks) == 0 {
		t.Fatal("/status produced no transcript block")
	}
	body := stripANSI(blockSearchText(s.blocks[len(s.blocks)-1]))
	if !strings.Contains(body, "sandbox: microsandbox (ready)") {
		t.Errorf("/status missing effective sandbox state:\n%s", body)
	}
	if !strings.Contains(body, "network: restricted") {
		t.Errorf("/status missing network mode:\n%s", body)
	}
}

// TestSandboxStatusModalReadyShowsNoSetup: a ready environment must not show
// setup actions (only the summary).
func TestSandboxStatusModalReadyShowsNoSetup(t *testing.T) {
	s := newSandboxSession(t)
	s.width, s.height = 100, 40
	s.openSandboxStatusModal()
	s.handleCoreEvent(rawEvent(t, "sandbox_status", map[string]any{
		"mode": "microsandbox",
		"report": map[string]any{
			"ready":     true,
			"supported": true,
			"platform":  "darwin",
			"checks": []any{
				map[string]any{"code": "virtualization", "title": "Virtualization", "status": "pass", "detail": ""},
			},
			"actions": []any{
				map[string]any{"title": "Unused action", "command": "should-not-appear"},
			},
		},
	}))
	body := stripANSI(s.renderModalBody())
	if !strings.Contains(body, "ready") {
		t.Errorf("ready status missing:\n%s", body)
	}
	if strings.Contains(body, "should-not-appear") {
		t.Errorf("ready modal must not show setup actions:\n%s", body)
	}
}
