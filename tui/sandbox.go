package main

import (
	"encoding/json"
	"fmt"
	"strings"

	tea "charm.land/bubbletea/v2"
	"github.com/atotto/clipboard"
)

// ---------------------------------------------------------------------------
// Sandbox subsystem (Microsandbox)
//
// CatCode sandboxes agent-controlled workloads inside a Microsandbox microVM
// (Linux KVM · Apple Silicon macOS · Windows Hypervisor Platform). The Rust
// core owns the runtime, preflight, and fail-closed enforcement; the TUI only
// renders status/setup guidance and issues the protocol commands
// (get_sandbox_status / prepare_sandbox / reset_sandbox).
//
// All platform/architecture detection is the core's responsibility — the TUI
// never gates the selector on runtime.GOOS, so Linux/macOS/Windows all present
// the same two options and react to the structured preflight report.
// ---------------------------------------------------------------------------

// normalizeSandboxValue maps a user-facing sandbox token to its canonical
// "none" | "microsandbox" form. Deprecated backends (firejail, seatbelt,
// sandbox-exec, …) are mapped to microsandbox and flagged so the caller can
// emit a one-time deprecation notice: the user's intent to enable sandboxing is
// preserved and never silently downgraded to "none". An unrecognized token
// returns ("", false).
func normalizeSandboxValue(v string) (mode string, deprecated bool) {
	switch strings.ToLower(strings.TrimSpace(v)) {
	case "", "none", "off", "false", "disabled":
		return "none", false
	case "microsandbox", "msb", "on", "true", "enabled":
		return "microsandbox", false
	case "firejail", "fj", "seatbelt", "macos", "sandbox-exec", "sandbox_exec":
		return "microsandbox", true
	default:
		return "", false
	}
}

// sandboxPreflightReport mirrors the core's structured preflight report. The
// TUI only renders this; it never interprets check codes into remediation text
// (the core is the source of truth for platform-specific guidance).
type sandboxPreflightReport struct {
	Requested    bool                    `json:"requested"`
	Supported    bool                    `json:"supported"`
	Ready        bool                    `json:"ready"`
	Platform     string                  `json:"platform"`
	Architecture string                  `json:"architecture"`
	Checks       []sandboxPreflightCheck `json:"checks"`
	Actions      []sandboxSetupAction    `json:"actions"`
}

// sandboxPreflightCheck is a single preflight check (KVM, WHP, runtime, image…).
type sandboxPreflightCheck struct {
	Code   string `json:"code"`
	Title  string `json:"title"`
	Status string `json:"status"` // pass | fail | warn | info
	Detail string `json:"detail"`
}

// sandboxSetupAction is a user-facing remediation step with an optional
// copyable command. RequiresAdmin/RequiresReboot mark steps CatCode will not
// perform automatically (they need elevated user action).
type sandboxSetupAction struct {
	Title          string `json:"title"`
	Explanation    string `json:"explanation"`
	Command        string `json:"command"`
	RequiresAdmin  bool   `json:"requires_admin"`
	RequiresReboot bool   `json:"requires_reboot"`
}

// sandboxStatusSnap is the TUI's latest view of the core's sandbox runtime
// state. It is populated incrementally from the `ready` event (summary fields)
// and the sandbox_status / sandbox_prepare_progress / sandbox_ready /
// sandbox_error events. It is distinct from settings.Sandbox (the desired mode
// the TUI persists + passes via --sandbox): this is the *effective* state the
// core resolved, which the TUI, web, CLI, and model prompt must agree on.
type sandboxStatusSnap struct {
	Mode         string // effective mode reported by core (none | microsandbox)
	Ready        bool
	Supported    bool
	Platform     string
	Architecture string
	Image        string
	Cpus         int
	MemoryMb     int
	NetworkMode  string
	Report       *sandboxPreflightReport
	PreparePhase string
	Error        string
	HasReport    bool
}

// parseSandboxReport decodes a report payload from a core event.
func parseSandboxReport(raw json.RawMessage) *sandboxPreflightReport {
	if len(raw) == 0 {
		return nil
	}
	var r sandboxPreflightReport
	if json.Unmarshal(raw, &r) != nil {
		return nil
	}
	return &r
}

// ensureSandboxStatus returns the mutable runtime snapshot, creating it if this
// is the first sandbox event. Callers assign back to s.sandboxStatus after
// mutating so the render cache never aliases a partially-updated struct.
func (s *session) ensureSandboxStatus() *sandboxStatusSnap {
	if s.sandboxStatus == nil {
		s.sandboxStatus = &sandboxStatusSnap{}
	}
	return s.sandboxStatus
}

// applyReadySandboxFields ingests the sandbox summary fields from the core's
// `ready` event. This is the effective state the core resolved at startup.
func (s *session) applyReadySandboxFields(m map[string]json.RawMessage) {
	snap := s.ensureSandboxStatus()
	if raw, ok := m["sandbox"]; ok {
		var v string
		if json.Unmarshal(raw, &v) == nil {
			norm, _ := normalizeSandboxValue(v)
			if norm == "" {
				norm = "none"
			}
			snap.Mode = norm
		}
	}
	if raw, ok := m["sandboxReady"]; ok {
		var b bool
		_ = json.Unmarshal(raw, &b)
		snap.Ready = b
	}
	if raw, ok := m["sandboxImage"]; ok {
		var v string
		_ = json.Unmarshal(raw, &v)
		snap.Image = v
	}
	if raw, ok := m["sandboxCpus"]; ok {
		var n int
		_ = json.Unmarshal(raw, &n)
		snap.Cpus = n
	}
	if raw, ok := m["sandboxMemoryMb"]; ok {
		var n int
		_ = json.Unmarshal(raw, &n)
		snap.MemoryMb = n
	}
	if raw, ok := m["sandboxNetworkMode"]; ok {
		var v string
		_ = json.Unmarshal(raw, &v)
		snap.NetworkMode = v
	}
	s.sandboxStatus = snap
}

// onSandboxStatusEvent handles `sandbox_status` {mode, report}: refresh the
// snapshot and resolve any pending enable request.
func (s *session) onSandboxStatusEvent(m map[string]json.RawMessage) {
	snap := s.ensureSandboxStatus()
	if mode := get(m, "mode"); mode != "" {
		norm, _ := normalizeSandboxValue(mode)
		if norm == "" {
			norm = mode
		}
		snap.Mode = norm
	}
	if raw, ok := m["report"]; ok {
		snap.Report = parseSandboxReport(raw)
		snap.HasReport = true
		if snap.Report != nil {
			snap.Ready = snap.Report.Ready
			snap.Supported = snap.Report.Supported
			snap.Platform = snap.Report.Platform
			snap.Architecture = snap.Report.Architecture
		}
	}
	snap.Error = ""
	s.sandboxStatus = snap
	s.setSandboxModalLoading(false)
	s.onSandboxStatusResolved()
}

// onSandboxPrepareProgressEvent handles `sandbox_prepare_progress` {phase}.
func (s *session) onSandboxPrepareProgressEvent(m map[string]json.RawMessage) {
	snap := s.ensureSandboxStatus()
	snap.PreparePhase = get(m, "phase")
	snap.Error = ""
	s.sandboxStatus = snap
	// Keep the modal in its loading/active state while preparation runs; it is
	// cleared by sandbox_ready / sandbox_error / a fresh status reply.
}

// onSandboxReadyEvent handles `sandbox_ready` {ready, report}.
func (s *session) onSandboxReadyEvent(m map[string]json.RawMessage) {
	snap := s.ensureSandboxStatus()
	snap.PreparePhase = ""
	if raw, ok := m["ready"]; ok {
		_ = json.Unmarshal(raw, &snap.Ready)
	}
	if raw, ok := m["report"]; ok {
		snap.Report = parseSandboxReport(raw)
		snap.HasReport = true
		if snap.Report != nil {
			snap.Supported = snap.Report.Supported
			snap.Platform = snap.Report.Platform
			snap.Architecture = snap.Report.Architecture
		}
	}
	s.sandboxStatus = snap
	s.setSandboxModalLoading(false)
	s.onSandboxStatusResolved()
}

// onSandboxErrorEvent handles `sandbox_error` {error}.
func (s *session) onSandboxErrorEvent(m map[string]json.RawMessage) {
	snap := s.ensureSandboxStatus()
	snap.Error = get(m, "error")
	snap.PreparePhase = ""
	// A hard error means the environment is not ready; cancel a pending enable
	// so the user must explicitly re-request it after fixing the setup.
	s.pendingSandboxEnable = false
	s.sandboxStatus = snap
	s.setSandboxModalLoading(false)
}

// onSandboxStatusResolved completes the pending-enable flow once the core has
// reported sandbox readiness. When the user asked to enable Microsandbox:
//   - ready     → persist the setting + offer a core restart;
//   - not ready → keep the status panel open with setup guidance; never save
//     "none" (that would claim a sandboxed session while commands still run on
//     the host) and never silently downgrade the user's intent.
func (s *session) onSandboxStatusResolved() {
	if !s.pendingSandboxEnable {
		return
	}
	s.pendingSandboxEnable = false
	snap := s.sandboxStatus
	if snap != nil && snap.Ready {
		s.settings.Sandbox = "microsandbox"
		_ = s.settings.save()
		s.logSuccess("sandbox: microsandbox (ready)")
		s.closeModal()
		s.offerCoreRestart("sandbox mode")
		return
	}
	s.logWarn("sandbox is not ready — review the setup steps below")
}

// --- protocol command requests -------------------------------------------

// sandboxEffectiveLabel renders the effective (core-reported) sandbox state
// for status displays (/status, diagnostics). It reflects what the core
// actually resolved — distinct from settings.Sandbox (the desired mode) — so
// the TUI, web, CLI, and model prompt all agree on whether commands run in
// the microVM.
func (s *session) sandboxEffectiveLabel() string {
	mode := "none"
	ready := false
	network := ""
	if s.sandboxStatus != nil {
		if s.sandboxStatus.Mode != "" {
			mode = s.sandboxStatus.Mode
		}
		ready = s.sandboxStatus.Ready
		network = s.sandboxStatus.NetworkMode
	}
	label := "disabled"
	if mode == "microsandbox" {
		if ready {
			label = "microsandbox (ready)"
		} else {
			label = "microsandbox (not ready)"
		}
	}
	if network != "" {
		label += " · network: " + network
	}
	return label
}

// requestSandboxStatus asks the core for a fresh preflight report and opens
// the status panel (loading until the reply arrives). Does not change the
// desired setting.
func (s *session) requestSandboxStatus() {
	s.pendingSandboxEnable = false
	s.openSandboxStatusModal()
	if s.sendCore(map[string]any{"type": "get_sandbox_status"}) {
		s.logInfo("checking sandbox environment…")
	}
}

// requestSandboxEnable begins the enable flow: ask the core for status, and on
// the reply persist "microsandbox" only if the environment is ready.
func (s *session) requestSandboxEnable() {
	s.pendingSandboxEnable = true
	s.openSandboxStatusModal()
	if s.sendCore(map[string]any{"type": "get_sandbox_status"}) {
		s.logInfo("checking sandbox environment…")
	}
}

// setSandboxNone disables sandboxing immediately (persisted + restart offered),
// mirroring how /sandbox none and the picker apply the change.
func (s *session) setSandboxNone() {
	s.settings.Sandbox = "none"
	_ = s.settings.save()
	s.logInfo("sandbox: none")
	s.offerCoreRestart("sandbox mode")
}

// requestSandboxPrepare asks the core to download/prepare user-space runtime
// + image assets. This is never an admin operation (no sudo, no feature
// enablement); admin steps are surfaced as copyable commands in the report.
func (s *session) requestSandboxPrepare() {
	if s.modal.kind != modalSandboxStatus {
		s.openSandboxStatusModal()
	}
	s.setSandboxModalLoading(true)
	if s.sendCore(map[string]any{"type": "prepare_sandbox"}) {
		s.logInfo("preparing sandbox runtime + image…")
	}
}

// requestSandboxReset asks the core to reset an unhealthy sandbox and opens
// the status panel so the reset result (sandbox_ready / sandbox_error /
// sandbox_status) is visible.
func (s *session) requestSandboxReset() {
	if s.modal.kind != modalSandboxStatus {
		s.openSandboxStatusModal()
	}
	s.setSandboxModalLoading(true)
	if s.sendCore(map[string]any{"type": "reset_sandbox"}) {
		s.logInfo("resetting sandbox…")
	}
}

// --- sandbox status modal -------------------------------------------------

// openSandboxStatusModal opens the scrollable sandbox status / setup panel.
func (s *session) openSandboxStatusModal() {
	s.modal = newModal()
	s.modal.kind = modalSandboxStatus
	s.modal.cursor = 0
	s.modal.scroll = 0
	s.modal.loading = s.sandboxStatus == nil || !s.sandboxStatus.HasReport
}

// setSandboxModalLoading toggles the loading spinner on the status modal only
// when it is the active modal, so an out-of-band status reply never disturbs a
// different modal the user has since opened.
func (s *session) setSandboxModalLoading(b bool) {
	if s.modal.kind == modalSandboxStatus {
		s.modal.loading = b
	}
}

// handleSandboxStatusKey drives the sandbox status panel: scroll the report,
// recheck, prepare user-space assets, and copy a setup command.
func (s *session) handleSandboxStatusKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "r":
		s.setSandboxModalLoading(true)
		if s.sendCore(map[string]any{"type": "get_sandbox_status"}) {
			s.logInfo("rechecking sandbox environment…")
		}
		return s, nil
	case "p":
		s.requestSandboxPrepare()
		return s, nil
	case "c":
		s.copyFirstSandboxCommand()
		return s, nil
	}
	switch {
	case s.kbAny(msg, "nav_up", "nav_up_alt"):
		if s.modal.scroll > 0 {
			s.modal.scroll--
		}
	case s.kbAny(msg, "nav_down", "nav_down_alt"):
		s.modal.scroll++
	case s.kb(msg, "scroll_page_up"):
		s.modal.scroll = max(0, s.modal.scroll-10)
	case s.kb(msg, "scroll_page_down"):
		s.modal.scroll += 10
	}
	return s, nil
}

// copyFirstSandboxCommand copies the first copyable setup command to the system
// clipboard (keyboard convenience; commands are also mouse-selectable).
func (s *session) copyFirstSandboxCommand() {
	snap := s.sandboxStatus
	if snap == nil || snap.Report == nil {
		s.logWarn("no setup command to copy yet")
		return
	}
	for _, a := range snap.Report.Actions {
		if strings.TrimSpace(a.Command) != "" {
			if err := clipboard.WriteAll(a.Command); err == nil {
				s.logSuccess("copied: " + a.Command)
				return
			}
			s.logWarn("system clipboard unavailable; select the command text to copy")
			return
		}
	}
	s.logWarn("no copyable setup command")
}

// renderSandboxStatusModal renders the structured status / setup panel.
func (s *session) renderSandboxStatusModal() string {
	w := s.modalWidth(96)
	snap := s.sandboxStatus

	mode := "none"
	ready := false
	if snap != nil {
		if snap.Mode != "" {
			mode = snap.Mode
		}
		ready = snap.Ready
	}
	modeLabel := "Disabled"
	if mode == "microsandbox" {
		modeLabel = "Microsandbox"
	}

	var lines []string
	if s.modal.loading {
		lines = append(lines, mutedStyle.Render("  ◷ Checking sandbox environment…"))
	} else {
		lines = append(lines, s.sandboxSummaryLines(snap, modeLabel, ready)...)
	}

	if snap != nil && snap.PreparePhase != "" {
		lines = append(lines, "", accentStyle.Render("  ⟳ Preparing: "+snap.PreparePhase+"…"))
	}
	if snap != nil && snap.Error != "" {
		lines = append(lines, "", errStyle.Render("  ✗ "+truncate(snap.Error, w-6)))
	}

	if snap != nil && snap.Report != nil {
		if len(snap.Report.Checks) > 0 {
			lines = append(lines, "", accentStyle.Render("  Preflight checks"))
			for _, ch := range snap.Report.Checks {
				lines = append(lines, "  "+sandboxCheckLine(ch))
			}
		}
		if !ready && len(snap.Report.Actions) > 0 {
			lines = append(lines, "", accentStyle.Render("  Setup required"))
			for _, a := range snap.Report.Actions {
				lines = append(lines, s.sandboxActionLines(a, w)...)
			}
		}
	}

	footer := "r recheck · p prepare · c copy command · ↑↓ scroll · esc close"
	allLines := wrapPlainReport(lines, max(1, w-4))
	return s.renderScrollableReport(w, "Sandbox Status", allLines, footer)
}

func (s *session) sandboxSummaryLines(snap *sandboxStatusSnap, modeLabel string, ready bool) []string {
	var lines []string
	readyStr := successStyle.Render("✓ ready")
	if snap != nil && snap.HasReport && !snap.Supported {
		readyStr = errStyle.Render("✗ unsupported on this platform")
	} else if !ready {
		readyStr = errStyle.Render("✗ not ready")
	}
	lines = append(lines, fmt.Sprintf("  Mode:      %s", modeLabel))
	if snap != nil {
		plat := snap.Platform
		if snap.Architecture != "" {
			if plat != "" {
				plat = plat + " · " + snap.Architecture
			} else {
				plat = snap.Architecture
			}
		}
		if plat != "" {
			lines = append(lines, fmt.Sprintf("  Platform:  %s", plat))
		}
	}
	lines = append(lines, fmt.Sprintf("  Ready:     %s", readyStr))
	if snap != nil {
		if snap.Image != "" {
			lines = append(lines, fmt.Sprintf("  Image:     %s", snap.Image))
		}
		if snap.Cpus > 0 {
			lines = append(lines, fmt.Sprintf("  CPUs:      %d", snap.Cpus))
		}
		if snap.MemoryMb > 0 {
			lines = append(lines, fmt.Sprintf("  Memory:    %d MB", snap.MemoryMb))
		}
		if snap.NetworkMode != "" {
			lines = append(lines, fmt.Sprintf("  Network:   %s", snap.NetworkMode))
		}
	}
	return lines
}

func sandboxCheckLine(ch sandboxPreflightCheck) string {
	icon := "•"
	style := mutedStyle
	switch strings.ToLower(ch.Status) {
	case "pass":
		icon, style = "✓", successStyle
	case "fail":
		icon, style = "✗", errStyle
	case "warn":
		icon, style = "⚠", warnStyle
	case "info":
		icon, style = "ℹ", accentStyle
	}
	title := ch.Title
	if title == "" {
		title = ch.Code
	}
	line := style.Render(icon) + " " + title
	if ch.Detail != "" {
		line += mutedStyle.Render(" — " + ch.Detail)
	}
	return line
}

func (s *session) sandboxActionLines(a sandboxSetupAction, w int) []string {
	var lines []string
	lines = append(lines, "  "+accentStyle.Render(a.Title))
	if a.Explanation != "" {
		lines = append(lines, "    "+mutedStyle.Render(truncate(a.Explanation, w-6)))
	}
	if cmd := strings.TrimSpace(a.Command); cmd != "" {
		lines = append(lines, "    "+codeInlineStyle.Render("$ "+truncate(cmd, w-8)))
	}
	var badges []string
	if a.RequiresAdmin {
		badges = append(badges, "requires admin")
	}
	if a.RequiresReboot {
		badges = append(badges, "requires reboot")
	}
	if len(badges) > 0 {
		lines = append(lines, "    "+warnStyle.Render("["+strings.Join(badges, " · ")+"]"))
	}
	return lines
}
