package main

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// ---------------------------------------------------------------------------
// Modal framework
//
// An overlay layer that renders on top of the viewport and intercepts keys.
// Four modal kinds share a filterable-list core; the settings modal adds
// editable/cyclable fields. Rendering is a centred bordered box via Place.
// ---------------------------------------------------------------------------

type modalKind int

const (
	modalNone modalKind = iota
	modalCommand
	modalModels
	modalSettings
	modalHelp
	modalTheme
	modalSessions
	modalPlugins
	modalReasoning
)

type modal struct {
	kind     modalKind
	filter   string // typed filter (list modals)
	cursor   int    // selected index in the filtered list
	scroll   int    // help modal vertical scroll
	fieldIdx int    // settings: active field
	editing  bool   // settings: a field is being edited
	editBuf  textinput.Model
}

// openReasoningPicker opens a list of the selected model's advertised
// thinking levels (falling back to low/medium/high) so the user can choose one
// directly instead of cycling.
func (s *session) openReasoningPicker() {
	s.modal = newModal()
	s.modal.kind = modalReasoning
	s.modal.cursor = 0
	levels := s.thinkingLevels()
	for i, l := range levels {
		if strings.EqualFold(l, s.settings.ReasoningEffort) {
			s.modal.cursor = i
			break
		}
	}
}

func newModal() modal {
	m := modal{}
	ti := textinput.New()
	ti.Prompt = ""
	m.editBuf = ti
	return m
}

// listItem is a generic filtered-list entry.
type listItem struct {
	label string
	desc  string
	tag   string // left marker (e.g. "▸" for selected)
}

// ---------------------------------------------------------------------------
// Open helpers
// ---------------------------------------------------------------------------

func (s *session) openCommandPalette() {
	s.modal = newModal()
	s.modal.kind = modalCommand
	s.modal.cursor = 0
}

func (s *session) openModelPicker() {
	s.modal = newModal()
	s.modal.kind = modalModels
	s.modal.cursor = s.modelIdx
}

func (s *session) openSettings() {
	s.modal = newModal()
	s.modal.kind = modalSettings
	s.modal.fieldIdx = 0
}

func (s *session) openHelp() {
	s.modal = newModal()
	s.modal.kind = modalHelp
	s.modal.scroll = 0
}

func (s *session) openSessionsPicker() {
	s.modal = newModal()
	s.modal.kind = modalSessions
	s.modal.cursor = 0
}

func (s *session) openPluginPicker(rawPlugins []json.RawMessage) {
	s.modal = newModal()
	s.modal.kind = modalPlugins
	s.modal.cursor = 0
	// Store plugin data in session (simple approach: use a field)
	sPluginStore = rawPlugins
}

var sPluginStore []json.RawMessage

// sessionItems builds the filtered-list entries for the session picker. The
// label is the human-readable title (derived from the first user message);
// the description shows the live message count and last-modified time, which
// update as the session grows. The active session is annotated.
func (s *session) sessionItems() []listItem {
	items := make([]listItem, len(s.sessionList))
	for i, e := range s.sessionList {
		label := truncateRunes(e.Title, 48)
		if e.Current {
			label = label + "  (current)"
		}
		desc := fmt.Sprintf("%d msgs · %s", e.Messages, formatMtime(e.Mtime))
		if e.Messages == 0 {
			desc = "empty · " + formatMtime(e.Mtime)
		}
		items[i] = listItem{label: label, desc: desc}
	}
	return items
}

func (s *session) openThemePicker() {
	s.modal = newModal()
	s.modal.kind = modalTheme
	s.modal.cursor = 0
	for i, t := range themes {
		if strings.EqualFold(t.name, s.settings.Theme) {
			s.modal.cursor = i
			break
		}
	}
}

func (s *session) closeModal() {
	s.modal.kind = modalNone
	s.modal.editing = false
}

// ---------------------------------------------------------------------------
// Filtered-list computation
// ---------------------------------------------------------------------------

func (s *session) commandItems() []listItem {
	return []listItem{
		{label: "/key", desc: "set API key"},
		{label: "/model", desc: "switch model"},
		{label: "/approval", desc: "never · destructive · always"},
		{label: "/reasoning", desc: "set reasoning effort (low/med/high)"},
		{label: "/reset", desc: "wipe conversation + session file"},
		{label: "/clear", desc: "clear view (keep session file)"},
		{label: "/undo", desc: "drop last turn"},
		{label: "/compact", desc: "force context compaction"},
		{label: "/sessions", desc: "open session picker"},
		{label: "/new", desc: "start a fresh session file"},
		{label: "/stats", desc: "token + turn totals"},
		{label: "/abort", desc: "stop running turn (or Esc)"},
		{label: "/steer", desc: "steer an in-flight turn (or Ctrl+Enter)"},
		{label: "/theme", desc: "switch colour theme"},
		{label: "/help", desc: "keybindings & commands"},
		{label: "/copy", desc: "copy last assistant reply"},
		{label: "/attach", desc: "attach an image (vision)"},
		{label: "/plugin-install", desc: "install a plugin from directory"},
		{label: "/plugin-list", desc: "list installed plugins"},
		{label: "/plugin-enable", desc: "enable a disabled plugin"},
		{label: "/plugin-disable", desc: "disable a plugin"},
		{label: "/plugin-remove", desc: "uninstall a plugin"},
		{label: "/run", desc: "delegate to a subagent (single)"},
		{label: "/parallel", desc: "run subagents in parallel"},
		{label: "/chain", desc: "run a subagent chain (->)"},
		{label: "/subagents", desc: "list available subagents"},
		{label: "/subagents-doctor", desc: "subagent setup diagnostics"},
		{label: "/subagents-status", desc: "show active subagent runs"},
	}
}

func (s *session) modelItems() []listItem {
	items := make([]listItem, len(s.models))
	for i, m := range s.models {
		// Show the model's advertised thinking levels when it constrains them
		// (e.g. GLM only takes "high"); omit the count for the standard trio.
		desc := fmt.Sprintf("ctx:%d · max:%d", m.ContextWindow, m.MaxTokens)
		if len(m.ThinkingLevels) > 0 {
			desc += " · think:" + strings.Join(m.ThinkingLevels, "/")
		}
		items[i] = listItem{
			label: m.ID,
			desc:  desc,
		}
	}
	return items
}

func (s *session) themeItems() []listItem {
	items := make([]listItem, len(themes))
	for i, t := range themes {
		items[i] = listItem{label: t.name, desc: ""}
	}
	return items
}

func (s *session) reasoningItems() []listItem {
	levels := s.thinkingLevels()
	items := make([]listItem, len(levels))
	for i, l := range levels {
		desc := ""
		if strings.EqualFold(l, s.settings.ReasoningEffort) {
			desc = "current"
		}
		items[i] = listItem{label: l, desc: desc}
	}
	return items
}

func (s *session) pluginItems() []listItem {
	var items []listItem
	for _, raw := range sPluginStore {
		var m map[string]json.RawMessage
		if json.Unmarshal(raw, &m) != nil {
			continue
		}
		name := get(m, "name")
		version := get(m, "version")
		desc := get(m, "description")
		enabled := get(m, "enabled")
		label := name + " v" + version
		if enabled == "false" {
			label = label + " (disabled)"
		}
		items = append(items, listItem{label: label, desc: desc})
	}
	if len(items) == 0 {
		items = append(items, listItem{label: "(no plugins installed)", desc: "use /plugin-install <dir> to add one"})
	}
	return items
}

// filterList returns the indices of items whose label or desc contains the
// filter (case-insensitive). Empty filter returns all.
func filterList(items []listItem, q string) []int {
	q = strings.ToLower(strings.TrimSpace(q))
	var idx []int
	for i, it := range items {
		if q == "" || strings.Contains(strings.ToLower(it.label), q) || strings.Contains(strings.ToLower(it.desc), q) {
			idx = append(idx, i)
		}
	}
	return idx
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

func (s *session) handleModalKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	// While editing a settings field, route keys to the edit buffer.
	if s.modal.editing {
		return s.handleSettingsEditKey(msg)
	}

	switch msg.String() {
	case "esc", "ctrl+c":
		if s.modal.kind != modalNone {
			s.closeModal()
			return s, nil
		}
	}

	switch s.modal.kind {
	case modalCommand, modalModels, modalTheme, modalSessions, modalPlugins, modalReasoning:
		return s.handleListKey(msg)
	case modalSettings:
		return s.handleSettingsKey(msg)
	case modalHelp:
		return s.handleHelpKey(msg)
	}
	return s, nil
}

func (s *session) handleListKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	var items []listItem
	switch s.modal.kind {
	case modalCommand:
		items = s.commandItems()
	case modalModels:
		items = s.modelItems()
	case modalTheme:
		items = s.themeItems()
	case modalSessions:
		items = s.sessionItems()
	case modalPlugins:
		items = s.pluginItems()
	case modalReasoning:
		items = s.reasoningItems()
	}
	idx := filterList(items, s.modal.filter)
	n := len(idx)

	switch msg.String() {
	case "up", "k":
		if n > 0 {
			s.modal.cursor = (s.modal.cursor - 1 + n) % n
		}
	case "down", "j":
		if n > 0 {
			s.modal.cursor = (s.modal.cursor + 1) % n
		}
	case "enter":
		if n == 0 {
			return s, nil
		}
		if s.modal.cursor >= n {
			s.modal.cursor = 0
		}
		abs := idx[s.modal.cursor]
		return s.executeListSelect(abs)
	case "backspace":
		if len(s.modal.filter) > 0 {
			r := []rune(s.modal.filter)
			s.modal.filter = string(r[:len(r)-1])
			s.modal.cursor = 0
		}
	case "ctrl+w":
		s.modal.filter = ""
		s.modal.cursor = 0
	default:
		// Printable chars accumulate into the filter.
		if isPrintable(msg) {
			s.modal.filter += msg.String()
			s.modal.cursor = 0
		}
	}
	return s, nil
}

// executeListSelect runs the action for the chosen absolute index.
func (s *session) executeListSelect(abs int) (tea.Model, tea.Cmd) {
	switch s.modal.kind {
	case modalCommand:
		s.closeModal()
		return s, s.runCommandByIndex(abs)
	case modalSessions:
		if abs >= 0 && abs < len(s.sessionList) {
			e := s.sessionList[abs]
			s.sendCore(map[string]any{"type": "load_session", "path": e.Path})
			s.logInfo("loading session: " + e.Title)
		}
		s.closeModal()
		return s, nil
	case modalModels:
		s.modelIdx = abs
		if abs >= 0 && abs < len(s.models) {
			s.settings.SelectedModel = s.models[abs].ID
			_ = s.settings.save()
			// Clamp reasoning effort to the newly selected model's thinking levels.
			if s.clampReasoning() {
				_ = s.settings.save()
				s.logInfo(fmt.Sprintf("reasoning: %s (for %s)", s.settings.ReasoningEffort, s.models[abs].ID))
			}
			s.logInfo(fmt.Sprintf("model: %s", s.models[abs].ID))
		}
		s.closeModal()
		return s, nil
	case modalTheme:
		if abs >= 0 && abs < len(themes) {
			setTheme(themes[abs].name)
			s.settings.Theme = themes[abs].name
			_ = s.settings.save()
			s.spinner.Style = lipgloss.NewStyle().Foreground(lipgloss.Color(c.accent))
			s.invalidateAll()
			s.refresh()
			s.logInfo("theme: " + themes[abs].name)
		}
		s.closeModal()
		return s, nil
	case modalPlugins:
		// Plugin picker is read-only; esc closes it.
		s.closeModal()
		return s, nil
	case modalReasoning:
		levels := s.thinkingLevels()
		if abs >= 0 && abs < len(levels) {
			s.settings.ReasoningEffort = levels[abs]
			_ = s.settings.save()
			s.logInfo(fmt.Sprintf("reasoning: %s", levels[abs]))
		}
		s.closeModal()
		return s, nil
	}
	s.closeModal()
	return s, nil
}

// runCommandByIndex maps a command-palette index to its action.
func (s *session) runCommandByIndex(i int) tea.Cmd {
	commands := s.commandItems()
	if i < 0 || i >= len(commands) {
		return nil
	}
	switch commands[i].label {
	case "/key":
		s.openSettings()
		s.modal.fieldIdx = 0
		s.startEditField(0)
		return nil
	case "/model":
		s.openModelPicker()
		return nil
	case "/approval":
		s.openSettings()
		s.modal.fieldIdx = 1
		return nil
	case "/reasoning":
		s.openReasoningPicker()
		return nil
	case "/reset":
		s.sendCore(map[string]any{"type": "reset"})
		s.blocks = nil
		s.cur = nil
		s.invalidateAll()
		s.viewport.SetContent("")
		return nil
	case "/abort":
		s.sendCore(map[string]any{"type": "abort"})
		return nil
	case "/settings":
		s.openSettings()
		return nil
	case "/theme":
		s.openThemePicker()
		return nil
	case "/help":
		s.openHelp()
		return nil
	case "/copy":
		return s.copyLastAssistant()
	default:
		// Any command not explicitly handled above (e.g. /sessions, /stats,
		// /clear, /undo, /compact, /attach) dispatches through handleUserLine
		// so the palette never needs a second dispatch table.
		return s.handleUserLine(commands[i].label)
	}
}

// ---------------------------------------------------------------------------
// Settings modal
// ---------------------------------------------------------------------------

type settingsField struct {
	label string
	value string
	hint  string
}

func (s *session) settingsFields() []settingsField {
	key := s.settings.APIKey
	if key != "" {
		if len(key) > 8 {
			key = key[:4] + "…" + key[len(key)-4:]
		} else {
			key = "…"
		}
	} else {
		key = "(not set)"
	}
	return []settingsField{
		{label: "API Key", value: key, hint: "enter to edit"},
		{label: "Approval", value: s.approvalMode(), hint: "enter to cycle"},
		{label: "Reasoning", value: s.settings.ReasoningEffort, hint: "enter to cycle"},
		{label: "Theme", value: activeTheme.name, hint: "enter to pick"},
		{label: "Bash Timeout", value: fmt.Sprintf("%ds", s.coreBashTimeout), hint: "enter to edit"},
		{label: "Sandbox", value: s.settings.Sandbox, hint: "enter to cycle"},
		{label: "No Network", value: boolStr(s.settings.NoNetwork), hint: "enter to toggle"},
		{label: "Idle Timeout", value: fmt.Sprintf("%ds", s.settings.IdleTimeout), hint: "enter to edit"},
		{label: "Max Session Tok", value: fmt.Sprintf("%d", s.settings.MaxSessionTokens), hint: "enter to edit"},
	}
}

func boolStr(b bool) string {
	if b {
		return "on"
	}
	return "off"
}

func (s *session) handleSettingsKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	fields := s.settingsFields()
	n := len(fields)
	switch msg.String() {
	case "up", "k":
		s.modal.fieldIdx = (s.modal.fieldIdx - 1 + n) % n
	case "down", "j", "tab":
		s.modal.fieldIdx = (s.modal.fieldIdx + 1) % n
	case "shift+tab":
		s.modal.fieldIdx = (s.modal.fieldIdx - 1 + n) % n
	case "enter":
		return s.activateField(s.modal.fieldIdx)
	case "left", "h":
		// cycle approval / reasoning left
		s.cycleField(s.modal.fieldIdx, -1)
	case "right", "l":
		s.cycleField(s.modal.fieldIdx, +1)
	}
	return s, nil
}

// activateField handles enter on a settings field.
func (s *session) activateField(idx int) (tea.Model, tea.Cmd) {
	fields := s.settingsFields()
	if idx < 0 || idx >= len(fields) {
		return s, nil
	}
	switch fields[idx].label {
	case "API Key":
		s.startEditField(idx)
		s.modal.editBuf.SetValue("")
		s.modal.editBuf.Placeholder = "sk-..."
		s.modal.editBuf.Focus()
		return s, textinput.Blink
	case "Approval":
		s.cycleApproval(+1)
	case "Reasoning":
		s.cycleReasoning(+1)
	case "Theme":
		s.openThemePicker()
	case "Bash Timeout":
		s.startEditField(idx)
		s.modal.editBuf.SetValue(fmt.Sprintf("%d", s.coreBashTimeout))
		s.modal.editBuf.Focus()
		return s, textinput.Blink
	case "Sandbox":
		s.cycleSandbox(+1)
	case "No Network":
		s.settings.NoNetwork = !s.settings.NoNetwork
		_ = s.settings.save()
		s.logInfo(fmt.Sprintf("no-network: %s (restarts core)", boolStr(s.settings.NoNetwork)))
	case "Idle Timeout":
		s.startEditField(idx)
		s.modal.editBuf.SetValue(fmt.Sprintf("%d", s.settings.IdleTimeout))
		s.modal.editBuf.Focus()
		return s, textinput.Blink
	case "Max Session Tok":
		s.startEditField(idx)
		s.modal.editBuf.SetValue(fmt.Sprintf("%d", s.settings.MaxSessionTokens))
		s.modal.editBuf.Focus()
		return s, textinput.Blink
	}
	return s, nil
}

func (s *session) cycleSandbox(dir int) {
	modes := []string{"none", "firejail"}
	cur := 0
	for i, m := range modes {
		if m == s.settings.Sandbox {
			cur = i
			break
		}
	}
	next := (cur + dir + len(modes)) % len(modes)
	s.settings.Sandbox = modes[next]
	_ = s.settings.save()
	s.logInfo(fmt.Sprintf("sandbox: %s (restarts core)", modes[next]))
}

func (s *session) startEditField(idx int) {
	s.modal.editing = true
	s.modal.fieldIdx = idx
	ti := textinput.New()
	ti.Prompt = ""
	s.modal.editBuf = ti
}

func (s *session) handleSettingsEditKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "esc":
		s.modal.editing = false
		return s, nil
	case "enter":
		return s.commitEditField()
	}
	var cmd tea.Cmd
	s.modal.editBuf, cmd = s.modal.editBuf.Update(msg)
	return s, cmd
}

func (s *session) commitEditField() (tea.Model, tea.Cmd) {
	fields := s.settingsFields()
	idx := s.modal.fieldIdx
	val := s.modal.editBuf.Value()
	s.modal.editing = false
	if idx < 0 || idx >= len(fields) {
		return s, nil
	}
	switch fields[idx].label {
	case "API Key":
		if strings.TrimSpace(val) != "" {
			s.settings.APIKey = strings.TrimSpace(val)
			_ = s.settings.save()
			s.sendCore(map[string]any{"type": "set_key", "api_key": s.settings.APIKey})
			s.logInfo("sending key…")
		}
	case "Bash Timeout":
		var n int
		if _, err := fmt.Sscanf(val, "%d", &n); err == nil && n > 0 {
			s.coreBashTimeout = n
			s.sendCore(map[string]any{"type": "set_config", "key": "bash_timeout_secs", "value": n})
		}
	case "Idle Timeout":
		var n int
		if _, err := fmt.Sscanf(val, "%d", &n); err == nil && n >= 10 {
			s.settings.IdleTimeout = n
			_ = s.settings.save()
			s.logInfo(fmt.Sprintf("idle timeout: %ds (restarts core)", n))
		}
	case "Max Session Tok":
		var n int
		if _, err := fmt.Sscanf(val, "%d", &n); err == nil && n >= 0 {
			s.settings.MaxSessionTokens = n
			_ = s.settings.save()
			s.logInfo(fmt.Sprintf("max session tokens: %d (restarts core)", n))
		}
	}
	return s, nil
}

// cycleField cycles a cyclable field (approval, reasoning) by dir (+1/-1).
func (s *session) cycleField(idx, dir int) {
	fields := s.settingsFields()
	if idx < 0 || idx >= len(fields) {
		return
	}
	switch fields[idx].label {
	case "Approval":
		s.cycleApproval(dir)
	case "Reasoning":
		s.cycleReasoning(dir)
	}
}

func (s *session) cycleApproval(dir int) {
	modes := []string{"never", "destructive", "always"}
	cur := 1
	for i, m := range modes {
		if m == s.approvalMode() {
			cur = i
			break
		}
	}
	next := (cur + dir + len(modes)) % len(modes)
	s.sendCore(map[string]any{"type": "set_approval", "mode": modes[next]})
	s.settings.Approval = modes[next]
	_ = s.settings.save()
}

func (s *session) cycleReasoning(dir int) {
	efforts := s.thinkingLevels()
	cur := 0
	for i, e := range efforts {
		if strings.EqualFold(e, s.settings.ReasoningEffort) {
			cur = i
			break
		}
	}
	next := (cur + dir + len(efforts)) % len(efforts)
	s.settings.ReasoningEffort = efforts[next]
	_ = s.settings.save()
}

// thinkingLevels returns the reasoning levels available for the currently
// selected model. Falls back to the standard low/medium/high set when the model
// advertises none (or no model is loaded yet), matching the core's default.
func (s *session) thinkingLevels() []string {
	if s.modelIdx >= 0 && s.modelIdx < len(s.models) {
		if lv := s.models[s.modelIdx].ThinkingLevels; len(lv) > 0 {
			return lv
		}
	}
	return []string{"low", "medium", "high"}
}

// clampReasoning keeps the persisted effort valid for the selected model's
// advertised thinking levels. If the current value is unsupported it picks the
// model's preferred level (high → medium → low → … → first). Returns true when
// the value changed (so callers can persist + notify). Mirrors the core's
// resolve_effort so the TUI display and the wire field always agree.
func (s *session) clampReasoning() bool {
	levels := s.thinkingLevels()
	cur := s.settings.ReasoningEffort
	if cur == "" {
		s.settings.ReasoningEffort = s.preferredLevel(levels)
		return true
	}
	for _, l := range levels {
		if strings.EqualFold(l, cur) {
			if l != cur { // normalize to the model's own casing
				s.settings.ReasoningEffort = l
				return true
			}
			return false
		}
	}
	s.settings.ReasoningEffort = s.preferredLevel(levels)
	return true
}

// preferredLevel picks the best-supported level from a set, preferring
// high → medium → low → minimal → none, then the first listed.
func (s *session) preferredLevel(levels []string) string {
	for _, pref := range []string{"high", "medium", "low", "minimal", "none"} {
		for _, l := range levels {
			if strings.EqualFold(l, pref) {
				return l
			}
		}
	}
	if len(levels) > 0 {
		return levels[0]
	}
	return "high"
}

// ---------------------------------------------------------------------------
// Help modal
// ---------------------------------------------------------------------------

func (s *session) handleHelpKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "up", "k":
		if s.modal.scroll > 0 {
			s.modal.scroll--
		}
	case "down", "j":
		s.modal.scroll++
	case "pgup":
		s.modal.scroll = max(0, s.modal.scroll-10)
	case "pgdown":
		s.modal.scroll += 10
	}
	return s, nil
}

func helpText() string {
	return strings.Join([]string{
		"Keybindings",
		"  ctrl+p / ctrl+k   open command palette",
		"  /                 command palette (when input empty)",
		"  @                 mention a file (CWD or @../ outside) — ↑↓ · tab",
		"                    select, esc closes the flyout",
		"  ctrl+t            toggle reasoning collapse",
		"  ctrl+o            expand / collapse last tool output",
		"  ctrl+r            set reasoning effort (low/med/high)",
		"  ctrl+c            quit",
		"  esc               close modal / deny approval / abort turn",
		"",
		"Scrolling the transcript",
		"  pgup / pgdn       scroll a page",
		"  ctrl+↑ / ctrl+↓   scroll a line",
		"  ctrl+home/end     jump to top / bottom",
		"  (scrolling up pauses auto-follow; sending a",
		"   message or reaching the bottom re-pins)",
		"",
		"While a turn is running (in-flight)",
		"  enter             queue a follow-up message",
		"  ctrl+enter        steer (interrupt + redirect the model)",
		"  esc               abort the turn",
		"  /steer <msg>      steer (works on every terminal)",
		"",
		"Approval (when prompted)",
		"  y                 approve once",
		"  a                 approve & stop asking",
		"  n                 deny",
		"",
		"Slash commands",
		"  /key sk-...       set API key",
		"  /model [N|substr] list or switch model",
		"  /approval <mode>  never | destructive | always",
		"  /reasoning        set reasoning effort (low/med/high)",
		"  /reset            wipe conversation + session file",
		"  /clear            clear view (keep session file)",
		"  /undo             drop last turn",
		"  /compact          force context compaction",
		"  /sessions         open session picker",
		"  /new              start a fresh session file",
		"  /stats            token + turn totals",
		"  /abort            stop running turn",
		"  /settings         open settings modal",
		"  /theme            switch colour theme",
		"  /copy             copy last assistant reply",
		"  /attach <path>   send an image (vision) with the current input",
		"",
		"Settings persist to ~/.config/umans-harness/settings.json",
		"Config (core) persists to ~/.config/umans-harness/config.json",
	}, "\n")
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

func (s *session) renderModalOverlay(base string) string {
	if s.modal.kind == modalNone {
		return base
	}
	box := s.renderModalBody()
	mw := lipgloss.Width(box)
	mh := lipgloss.Height(box)
	w := s.width
	h := s.height
	if mw > w-2 {
		mw = w - 2
	}
	if mh > h-2 {
		mh = h - 2
	}
	// Overlay: place the box over the base via centered placement.
	overlay := lipgloss.Place(w, h, lipgloss.Center, lipgloss.Center, box)
	return overlay
}

func (s *session) renderModalBody() string {
	switch s.modal.kind {
	case modalCommand:
		return s.renderListModal("Command Palette", s.commandItems(), true)
	case modalModels:
		return s.renderListModal("Models", s.modelItems(), true)
	case modalTheme:
		return s.renderListModal("Theme", s.themeItems(), false)
	case modalSessions:
		return s.renderListModal("Sessions", s.sessionItems(), true)
	case modalPlugins:
		return s.renderListModal("Plugins", s.pluginItems(), false)
	case modalReasoning:
		return s.renderListModal("Reasoning Effort", s.reasoningItems(), true)
	case modalSettings:
		return s.renderSettingsModal()
	case modalHelp:
		return s.renderHelpModal()
	}
	return ""
}

func (s *session) renderListModal(title string, items []listItem, showFilter bool) string {
	w := 52
	if s.width-4 < w {
		w = s.width - 4
	}
	if w < 28 {
		w = 28
	}
	idx := filterList(items, s.modal.filter)
	n := len(idx)

	// visible window: cap so long lists scroll instead of overflowing.
	maxVisible := s.height - 9 // title+filter+sep+footer+border padding
	if maxVisible < 4 {
		maxVisible = 4
	}
	if n > maxVisible {
		// keep the cursor inside the window
		if s.modal.cursor < s.modal.scroll {
			s.modal.scroll = s.modal.cursor
		} else if s.modal.cursor >= s.modal.scroll+maxVisible {
			s.modal.scroll = s.modal.cursor - maxVisible + 1
		}
	} else {
		s.modal.scroll = 0
	}
	rowW := w - 4 // modal border(2) + padding(2)
	if rowW < 1 {
		rowW = 1
	}
	hiStyle := lipgloss.NewStyle().
		Background(lipgloss.Color(c.dim)).
		Foreground(lipgloss.Color(c.fg)).
		Width(rowW)

	var lines []string
	lines = append(lines, accentStyle.Render("◆ "+title))
	if showFilter {
		fq := s.modal.filter
		if fq == "" {
			fq = dimStyle.Render("type to filter…")
		}
		lines = append(lines, inputPromptStyle.Render("⟩ ")+mutedStyle.Render(fq))
	}
	lines = append(lines, separatorStyle.Render(strings.Repeat("─", w-2)))
	if n == 0 {
		lines = append(lines, dimStyle.Render("  (no matches)"))
	}
	visStart := s.modal.scroll
	visEnd := visStart + maxVisible
	if visEnd > n {
		visEnd = n
	}
	for vi := visStart; vi < visEnd; vi++ {
		abs := idx[vi]
		marker := "  "
		if vi == s.modal.cursor {
			marker = accentStyle.Render("▸ ")
		}
		label := baseStyle.Render(items[abs].label)
		desc := ""
		if items[abs].desc != "" {
			desc = "  " + dimStyle.Render(items[abs].desc)
		}
		row := marker + label + desc
		if vi == s.modal.cursor {
			row = hiStyle.Render(row) // full-row highlight bar
		}
		lines = append(lines, row)
	}
	if n > maxVisible {
		lines = append(lines, dimStyle.Render(fmt.Sprintf("  (%d more · ↑↓ scroll)", n-maxVisible)))
	}
	lines = append(lines, "")
	lines = append(lines, dimStyle.Render("  ↑↓ navigate · enter select · esc close"))
	body := strings.Join(lines, "\n")
	return modalBox(w, body)
}

func (s *session) renderSettingsModal() string {
	w := 52
	if s.width-4 < w {
		w = s.width - 4
	}
	if w < 30 {
		w = 30
	}
	fields := s.settingsFields()
	var lines []string
	lines = append(lines, accentStyle.Render("◆ Settings"))
	lines = append(lines, separatorStyle.Render(strings.Repeat("─", w-2)))
	for i, f := range fields {
		marker := "  "
		if i == s.modal.fieldIdx {
			marker = accentStyle.Render("▸ ")
		}
		label := baseStyle.Render(f.label)
		val := mutedStyle.Render(f.value)
		if s.modal.editing && i == s.modal.fieldIdx {
			val = accentStyle.Render("[") + s.modal.editBuf.View() + accentStyle.Render("]")
		}
		hint := "  " + dimStyle.Render(f.hint)
		lines = append(lines, marker+label+": "+val+hint)
	}
	lines = append(lines, "")
	lines = append(lines, dimStyle.Render("  ↑↓ navigate · enter edit/apply · ←→ cycle · esc close"))
	body := strings.Join(lines, "\n")
	return modalBox(w, body)
}

func (s *session) renderHelpModal() string {
	w := 60
	if s.width-4 < w {
		w = s.width - 4
	}
	if w < 30 {
		w = 30
	}
	h := s.height - 6
	if h < 6 {
		h = 6
	}
	allLines := strings.Split(helpText(), "\n")
	maxScroll := len(allLines) - h
	if maxScroll < 0 {
		maxScroll = 0
	}
	if s.modal.scroll > maxScroll {
		s.modal.scroll = maxScroll
	}
	if s.modal.scroll < 0 {
		s.modal.scroll = 0
	}
	start := s.modal.scroll
	end := start + h
	if end > len(allLines) {
		end = len(allLines)
	}
	visible := strings.Join(allLines[start:end], "\n")
	body := accentStyle.Render("◆ Help") + "\n" + visible + "\n" + dimStyle.Render("  ↑↓ scroll · esc close")
	return modalBox(w, body)
}

// modalBox wraps a body in a rounded border with padding.
func modalBox(w int, body string) string {
	return lipgloss.NewStyle().
		BorderStyle(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(c.accent)).
		Padding(0, 1).
		Width(w).
		Render(body)
}

func isPrintable(msg tea.KeyMsg) bool {
	r := []rune(msg.String())
	if len(r) != 1 {
		return false
	}
	c := r[0]
	return c >= 0x20 && c != 0x7f
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}
