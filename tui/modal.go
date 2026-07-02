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
	modalVision
	modalProviders
	modalLogout
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
	meta  string // opaque payload for executeListSelect (e.g. preset id)
	meta2 string // opaque kind hint for executeListSelect (e.g. "preset"/"provider")
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

// openVisionPicker opens the vision-models modal. The list comes from the
// discovered models (s.models); per-row vision state merges each model's
// endpoint Vision flag with the /vision-curated set (s.visionModels), and the
// preferred handoff target is s.visionModel (★).
func (s *session) openVisionPicker() {
	s.modal = newModal()
	s.modal.kind = modalVision
	s.modal.cursor = 0
}

// openLoginPicker opens the /login picker. It lists the first-party presets
// (OpenAI/Codex, Gemini, Anthropic) plus any other configured providers, so
// the user can log in to one or switch to an already-logged-in one. Logging in
// to a preset whose key is in the env just sends `login`; one with no key
// prompts the user to paste a key inline, then sends `login {preset,api_key}`.
// Multiple providers can be logged in at once; their models all appear in
// /models and each turn routes to the selected model's provider.
func (s *session) openLoginPicker() {
	s.modal = newModal()
	s.modal.kind = modalProviders
	s.modal.cursor = 0
	s.pendingLogin = ""
	// Make sure we have the latest preset list (configured/hasKey/loggedIn flags).
	s.sendCore(map[string]any{"type": "list_provider_presets"})
}

// openLogoutPicker opens the /logout picker: lists only providers that are
// currently logged in (presets with LoggedIn + configured non-preset providers
// with a key). Selecting one sends `logout` and re-aggregates models.
func (s *session) openLogoutPicker() {
	s.modal = newModal()
	s.modal.kind = modalLogout
	s.modal.cursor = 0
	s.pendingLogin = ""
}

// providerItems builds the /login picker list: first-party presets followed by
// any other configured providers. `meta` carries the preset/provider id and
// `meta2` the kind ("preset"/"provider") so selectProviderItem can dispatch.
func (s *session) providerItems() []listItem {
	items := make([]listItem, 0, len(s.providerPresets)+len(s.providers))
	for _, p := range s.providerPresets {
		label := p.Label
		desc := p.Description
		switch {
		case p.LoggedIn:
			label = "✓ " + p.Label
			if p.ID == s.activeProvider {
				desc = "logged in · active · enter to override key (empty = switch) · " + desc
			} else {
				desc = "logged in · enter to override key (empty = switch) · " + desc
			}
		case p.HasKey:
			label = "▸ " + p.Label
			desc = "ready (key in " + p.EnvVar + ") · enter to log in · " + desc
		default:
			label = "▸ " + p.Label
			desc = "enter key to log in · needs " + p.EnvVar + " · " + desc
		}
		items = append(items, listItem{label: label, desc: desc, meta: p.ID, meta2: "preset"})
	}
	// Configured providers not covered by a preset (e.g. custom/local).
	for _, name := range s.providers {
		if s.presetByID(name) != nil {
			continue
		}
		label := name
		if name == s.activeProvider {
			label = name + "  (active)"
		}
		items = append(items, listItem{label: label, desc: "switch · configured", meta: name, meta2: "provider"})
	}
	return items
}

// logoutItems builds the /logout picker list: only providers that are logged in.
func (s *session) logoutItems() []listItem {
	items := make([]listItem, 0, len(s.providerPresets)+len(s.providers))
	for _, p := range s.providerPresets {
		if !p.LoggedIn {
			continue
		}
		label := p.Label
		if p.ID == s.activeProvider {
			label = p.Label + "  (active)"
		}
		items = append(items, listItem{label: label, desc: "log out", meta: p.ID, meta2: "preset"})
	}
	for _, name := range s.providers {
		if s.presetByID(name) != nil {
			continue
		}
		// Non-preset configured providers: include if it has a persisted key.
		if s.providerKey(name) == "" {
			continue
		}
		label := name
		if name == s.activeProvider {
			label = name + "  (active)"
		}
		items = append(items, listItem{label: label, desc: "log out", meta: name, meta2: "provider"})
	}
	return items
}

// presetByID returns the matching preset for an id, or nil.
func (s *session) presetByID(id string) *providerPreset {
	for i := range s.providerPresets {
		if s.providerPresets[i].ID == id {
			return &s.providerPresets[i]
		}
	}
	return nil
}

func (s *session) visionItems() []listItem {
	items := make([]listItem, len(s.models))
	for i, m := range s.models {
		on := m.Vision || s.visionModels[m.ID]
		check := " "
		if on {
			check = "x"
		}
		star := "  "
		if s.visionModel == m.ID {
			star = "★ "
		}
		label := fmt.Sprintf("[%s] %s%s", check, star, m.ID)
		desc := ""
		if m.Vision {
			desc = "endpoint vision"
		}
		items[i] = listItem{label: label, desc: desc}
	}
	return items
}

// saveVisionConfig persists the current vision config (curated set + preferred
// target) to the core, which writes .umans-harness/vision.json and echoes a
// vision_config event that re-syncs the TUI state. Empty vision_model => the
// core treats it as None (pick dynamically).
func (s *session) saveVisionConfig() {
	vm := make([]string, 0, len(s.visionModels))
	for id, on := range s.visionModels {
		if on {
			vm = append(vm, id)
		}
	}
	s.sendCore(map[string]any{
		"type":          "set_vision_config",
		"vision_models": vm,
		"vision_model":  s.visionModel,
	})
}

// handleVisionKey drives the vision picker: space toggles vision-capable for
// the highlighted model, enter sets/clears the preferred handoff target (★).
// Both live-persist via saveVisionConfig; the modal stays open. Filter typing
// works like the other list modals.
func (s *session) handleVisionKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	items := s.visionItems()
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
	case " ":
		if n > 0 && s.modal.cursor < n {
			abs := idx[s.modal.cursor]
			if abs < len(s.models) {
				id := s.models[abs].ID
				s.visionModels[id] = !s.visionModels[id]
				if !s.visionModels[id] && s.visionModel == id {
					s.visionModel = "" // can't be the target if not vision-capable
				}
				s.saveVisionConfig()
			}
		}
	case "enter":
		if n > 0 && s.modal.cursor < n {
			abs := idx[s.modal.cursor]
			if abs < len(s.models) {
				id := s.models[abs].ID
				if s.visionModel == id {
					s.visionModel = "" // toggle off → dynamic pick
				} else {
					s.visionModels[id] = true // a target must be vision-capable
					s.visionModel = id
				}
				s.saveVisionConfig()
			}
		}
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
		if isPrintable(msg) {
			s.modal.filter += msg.String()
			s.modal.cursor = 0
		}
	}
	return s, nil
}

// sessionItems builds the filtered-list entries for the session picker. The
// label is the human-readable title (derived from the first user message);
// the description shows the live message count and last-modified time, which
// update as the session grows. The active session is annotated.
func (s *session) sessionItems() []listItem {
	items := make([]listItem, len(s.sessionList))
	for i, e := range s.sessionList {
		label := truncateRunes(e.Title, 200) // fitListRow truncates to the actual row width
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
	items := []listItem{
		{label: "/login", desc: "log in / switch provider (OpenAI · Gemini · Anthropic)"},
		{label: "/logout", desc: "log out of a provider"},
		{label: "/model", desc: "switch model"},
		{label: "/approval", desc: "never · destructive · always"},
		{label: "/reasoning", desc: "set reasoning effort (per model)"},
		{label: "/reset", desc: "wipe conversation + session file"},
		{label: "/clear", desc: "clear view (keep session file)"},
		{label: "/undo", desc: "drop last turn"},
		{label: "/compact", desc: "force context compaction"},
		{label: "/sessions", desc: "open session picker"},
		{label: "/new", desc: "start a fresh session file"},
		{label: "/stats", desc: "token + turn totals"},
		{label: "/abort", desc: "stop running turn (or Esc)"},
		{label: "/steer", desc: "steer an in-flight turn (or Ctrl+Enter)"},
		{label: "/settings", desc: "open settings modal"},
		{label: "/theme", desc: "switch colour theme"},
		{label: "/help", desc: "keybindings & commands"},
		{label: "/copy", desc: "copy last assistant reply"},
		{label: "/attach", desc: "attach an image (vision)"},
		{label: "/vision", desc: "configure vision models & handoff target"},
		{label: "/plugin-install", desc: "install a plugin from directory"},
		{label: "/plugin-config", desc: "list plugins · enter to enable/disable"},
		{label: "/plugin-remove", desc: "uninstall a plugin"},
		{label: "/run", desc: "delegate to a subagent (single)"},
		{label: "/parallel", desc: "run subagents in parallel"},
		{label: "/chain", desc: "run a subagent chain (->)"},
		{label: "/subagents", desc: "list available subagents"},
		{label: "/subagents-doctor", desc: "subagent setup diagnostics"},
		{label: "/subagents-status", desc: "show active subagent runs"},
		{label: "/remember", desc: "save a memory note (persisted across sessions)"},
		{label: "/memory", desc: "list saved memories"},
		{label: "/forget", desc: "forget a memory by id"},
		{label: "/index", desc: "bootstrap repo knowledge → memories + candidate skills"},
		{label: "/reflect", desc: "reflect on this session, persist durable learnings"},
	}
	// Append one /skill:<name> entry per discoverable skill so skills created
	// manually or by the learning system are invocable from the palette with
	// autocomplete like the built-in commands.
	for _, sk := range s.skillsList {
		desc := sk.Description
		if desc == "" {
			desc = "apply skill"
		}
		items = append(items, listItem{label: "/skill:" + sk.Name, desc: desc})
	}
	return items
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
		// Tag the owning provider so a multi-login /models can mix providers
		// (e.g. gpt-5-codex [openai], gemini-2.5-pro [gemini], claude-... [anthropic]).
		label := m.ID
		if m.Provider != "" {
			label = fmt.Sprintf("%s  [%s]", m.ID, m.Provider)
		}
		items[i] = listItem{
			label: label,
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
		action := "disable"
		if enabled == "false" {
			label += " (disabled)"
			action = "enable"
		} else {
			label += " (enabled)"
		}
		hint := "enter to " + action
		if desc != "" {
			hint = desc + " · " + hint
		}
		items = append(items, listItem{label: label, desc: hint})
	}
	if len(items) == 0 {
		items = append(items, listItem{label: "(no plugins installed)", desc: "use /plugin-install <dir> to add one"})
	}
	return items
}

// togglePlugin flips the enabled state of the plugin at store index idx. It
// sends the matching core command (enable_plugin / disable_plugin) and
// optimistically updates the cached store so the picker re-renders the new
// state immediately, without waiting for a fresh list_plugins round-trip.
func (s *session) togglePlugin(idx int) {
	if idx < 0 || idx >= len(sPluginStore) {
		return
	}
	var m map[string]any
	if json.Unmarshal(sPluginStore[idx], &m) != nil {
		return
	}
	name, _ := m["name"].(string)
	enabled, _ := m["enabled"].(bool)
	if name == "" {
		return
	}
	if enabled {
		s.sendCore(map[string]any{"type": "disable_plugin", "name": name})
		m["enabled"] = false
	} else {
		s.sendCore(map[string]any{"type": "enable_plugin", "name": name})
		m["enabled"] = true
	}
	if raw, err := json.Marshal(m); err == nil {
		sPluginStore[idx] = raw
	}
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
	case modalCommand, modalModels, modalTheme, modalSessions, modalPlugins, modalReasoning, modalProviders, modalLogout:
		return s.handleListKey(msg)
	case modalVision:
		return s.handleVisionKey(msg)
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
	case modalProviders:
		items = s.providerItems()
	case modalLogout:
		items = s.logoutItems()
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
		// Enter toggles the selected plugin's enabled state; the modal
		// stays open so several can be toggled in one visit.
		s.togglePlugin(abs)
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
	case modalProviders:
		return s.selectProviderItem(abs)
	case modalLogout:
		return s.selectLogoutItem(abs)
	}
	s.closeModal()
	return s, nil
}

// selectProviderItem handles a pick in the provider modal: a preset entry adds
// the first-party provider (add_provider), a configured-provider entry switches
// to it (set_provider). The modal closes on add; on switch it relies on the
// core's provider_changed event to update state (mirrors cycleProvider).
func (s *session) selectProviderItem(abs int) (tea.Model, tea.Cmd) {
	items := s.providerItems()
	if abs < 0 || abs >= len(items) {
		return s, nil
	}
	it := items[abs]
	switch it.meta2 {
	case "preset":
		name := it.meta
		preset := s.presetByID(name)
		if preset == nil {
			s.closeModal()
			return s, nil
		}
		// Already logged in: let the user OVERRIDE the key (e.g. fix a bad env
		// var that caused a 401). Opens the inline key-entry box; an empty submit
		// just switches to it instead of overriding. A pasted key replaces the
		// provider's config with a literal key that takes precedence over the
		// env var and is persisted, so the override survives restarts.
		if preset.LoggedIn {
			s.pendingLogin = name
			s.modal.editing = true
			s.modal.editBuf.SetValue("")
			s.modal.editBuf.Placeholder = "paste new key to override (empty = just switch)"
			s.modal.editBuf.Focus()
			s.modal.editBuf.CursorEnd()
			return s, nil
		}
		// A key is available from the env var: log in immediately, no prompt.
		if preset.HasKey {
			s.sendCore(map[string]any{"type": "login", "preset": name})
			s.logInfo("logging in to " + preset.Label)
			s.closeModal()
			return s, nil
		}
		// No key anywhere: prompt the user to paste one inline. The modal
		// switches to the key-entry box (renderLoginKeyBox) and the next Enter
		// sends `login {preset,api_key}` via the editing-key handler.
		s.pendingLogin = name
		s.modal.editing = true
		s.modal.editBuf.SetValue("")
		s.modal.editBuf.Placeholder = "paste " + preset.EnvVar + " value"
		s.modal.editBuf.Focus()
		s.modal.editBuf.CursorEnd()
		return s, nil
	case "provider":
		name := it.meta
		s.settings.ActiveProvider = name
		_ = s.settings.save()
		s.sendCore(map[string]any{"type": "set_provider", "name": name})
		s.logInfo("switching provider: " + name)
		s.closeModal()
		return s, nil
	}
	s.closeModal()
	return s, nil
}

// selectLogoutItem handles a pick in the /logout modal: send `logout` for the
// chosen provider, then drop its persisted key on the TUI side and save. The
// core re-aggregates models (refresh_models) so the provider's models vanish.
func (s *session) selectLogoutItem(abs int) (tea.Model, tea.Cmd) {
	items := s.logoutItems()
	if abs < 0 || abs >= len(items) {
		return s, nil
	}
	it := items[abs]
	name := it.meta
	s.sendCore(map[string]any{"type": "logout", "provider": name})
	s.deleteProviderKey(name)
	if s.settings.ActiveProvider == name {
		s.settings.ActiveProvider = ""
	}
	_ = s.settings.save()
	s.logInfo("logged out of " + name)
	s.closeModal()
	return s, nil
}

// renderLoginKeyBox renders the inline API-key entry box used by the /login
// modal when a preset has no key in the environment. Mirrors the settings
// modal's secret-field rendering (masked).
func (s *session) renderLoginKeyBox() string {
	label := "API Key"
	if p := s.presetByID(s.pendingLogin); p != nil {
		label = p.Label + " API Key (" + p.EnvVar + ")"
	}
	val := s.modal.editBuf.Value()
	masked := strings.Repeat("•", len(val))
	return s.renderListModal("Log in: "+label, []listItem{{
		label: masked,
		desc: "paste your key, then Enter (Esc to cancel)",
	}}, true)
}
func (s *session) runCommandByIndex(i int) tea.Cmd {
	commands := s.commandItems()
	if i < 0 || i >= len(commands) {
		return nil
	}
	label := commands[i].label
	// /skill:<name> — insert into the input box (with a trailing space) instead
	// of dispatching immediately, so the user can append a task message and send
	// them as one turn. Press Enter again to run the bare skill with no task.
	// Other commands are instant actions (they take no inline argument), so
	// they still dispatch right away.
	if strings.HasPrefix(label, "/skill:") {
		s.closeModal()
		s.input.SetValue(label + " ")
		s.input.CursorEnd()
		s.evalMention()
		return s.input.Focus()
	}
	switch label {
	case "/login":
		s.openLoginPicker()
		return nil
	case "/logout":
		s.openLogoutPicker()
		return nil
	case "/model":
		s.openModelPicker()
		return nil
	case "/approval":
		s.openSettings()
		s.modal.fieldIdx = s.settingsFieldIndex("Approval")
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
		{label: "Provider", value: s.providerFieldLabel(), hint: s.providerFieldHint()},
		{label: "API Key", value: key, hint: "enter to edit"},
		{label: "Approval", value: s.approvalMode(), hint: "enter to cycle"},
		{label: "Reasoning", value: s.settings.ReasoningEffort, hint: "enter to cycle"},
		{label: "Theme", value: activeTheme.name, hint: "enter to pick"},
		{label: "Bash Timeout", value: fmt.Sprintf("%ds", s.coreBashTimeout), hint: "enter to edit"},
		{label: "Sandbox", value: s.settings.Sandbox, hint: "enter to cycle"},
		{label: "No Network", value: boolStr(s.settings.NoNetwork), hint: "enter to toggle"},
		{label: "Mouse Wheel", value: boolStr(s.settings.MouseWheel), hint: "enter to toggle"},
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

// settingsFieldIndex returns the index of the settings field whose label
// matches, or -1 if none. Palette shortcuts (/key, /approval) use this so
// they target the correct row regardless of the field ordering in
// settingsFields() — hard-coded indices broke when "Provider" was added at
// index 0 (e.g. /key landed on Provider instead of API Key, and the typed
// key was then dropped on commit since Provider has no edit handler).
func (s *session) settingsFieldIndex(label string) int {
	for i, f := range s.settingsFields() {
		if f.label == label {
			return i
		}
	}
	return -1
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
	case "Provider":
		s.cycleProvider(+1)
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
		s.logInfo(fmt.Sprintf("no-network: %s (applies on next launch)", boolStr(s.settings.NoNetwork)))
	case "Mouse Wheel":
		s.settings.MouseWheel = !s.settings.MouseWheel
		_ = s.settings.save()
		if s.settings.MouseWheel {
			s.logInfo("mouse wheel: on (hold Shift to select/copy text)")
			return s, tea.EnableMouseCellMotion
		}
		s.logInfo("mouse wheel: off (click-drag to select/copy text)")
		return s, tea.DisableMouse
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
	s.logInfo(fmt.Sprintf("sandbox: %s (applies on next launch)", modes[next]))
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
	// /login inline key entry: a preset was picked with no env key, so the
	// modal captured a pasted key in the edit buffer. Commit sends `login`
	// with that key and closes the modal.
	if s.pendingLogin != "" {
		name := s.pendingLogin
		key := strings.TrimSpace(s.modal.editBuf.Value())
		s.pendingLogin = ""
		s.modal.editing = false
		if key == "" {
			// Empty submit: if the provider is already logged in, treat as a
			// switch (no override); otherwise cancel.
			if p := s.presetByID(name); p != nil && p.LoggedIn {
				s.settings.ActiveProvider = name
				_ = s.settings.save()
				s.sendCore(map[string]any{"type": "set_provider", "name": name})
				s.logInfo("switching to " + p.Label)
				s.closeModal()
				return s, nil
			}
			s.logError("no key entered; cancelled login")
			s.closeModal()
			return s, nil
		}
		// Persist the key on the TUI side so it survives restart.
		if s.settings.ProviderKeys == nil {
			s.settings.ProviderKeys = map[string]string{}
		}
		s.settings.ProviderKeys[name] = key
		_ = s.settings.save()
		if p := s.presetByID(name); p != nil {
			s.logInfo("logging in to " + p.Label)
		}
		s.sendCore(map[string]any{"type": "login", "preset": name, "api_key": key})
		s.closeModal()
		return s, nil
	}
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
			key := strings.TrimSpace(val)
			// Scope the key to the active provider (per-provider keys).
			name := s.activeProvider
			if name == "" {
				name = "default"
			}
			if s.settings.ProviderKeys == nil {
				s.settings.ProviderKeys = map[string]string{}
			}
			s.settings.ProviderKeys[name] = key
			s.settings.APIKey = key
			_ = s.settings.save()
			s.sendCore(map[string]any{"type": "set_key", "provider": name, "api_key": key})
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
			s.logInfo(fmt.Sprintf("idle timeout: %ds (applies on next launch)", n))
		}
	case "Max Session Tok":
		var n int
		if _, err := fmt.Sscanf(val, "%d", &n); err == nil && n >= 0 {
			s.settings.MaxSessionTokens = n
			_ = s.settings.save()
			s.logInfo(fmt.Sprintf("max session tokens: %d (applies on next launch)", n))
		}
	}
	return s, nil
}

// cycleField cycles a cyclable field (approval, reasoning, provider) by dir (+1/-1).
func (s *session) cycleField(idx, dir int) {
	fields := s.settingsFields()
	if idx < 0 || idx >= len(fields) {
		return
	}
	switch fields[idx].label {
	case "Provider":
		s.cycleProvider(dir)
	case "Approval":
		s.cycleApproval(dir)
	case "Reasoning":
		s.cycleReasoning(dir)
	}
}

// providerFieldLabel renders the active provider's name + kind for the settings
// modal (e.g. "anthropic [anthropic]"). Shows "default" when none configured.
func (s *session) providerFieldLabel() string {
	name := s.activeProvider
	if name == "" {
		name = "default"
	}
	if s.providerKind != "" {
		return fmt.Sprintf("%s [%s]", name, s.providerKind)
	}
	return name
}

// providerFieldHint tells the user what enter/cycle does; "(configured in
// config.json)" when no custom providers are defined (nothing to cycle).
func (s *session) providerFieldHint() string {
	if len(s.providers) > 0 {
		return "←→ cycle · enter apply"
	}
	return "configured in config.json"
}

// cycleProvider switches to the next/previous configured provider and tells
// the core to switch (re-discovers models + re-resolves the key). No-op when
// no providers are configured.
func (s *session) cycleProvider(dir int) {
	if len(s.providers) == 0 {
		return
	}
	cur := 0
	for i, p := range s.providers {
		if p == s.activeProvider {
			cur = i
			break
		}
	}
	next := (cur + dir + len(s.providers)) % len(s.providers)
	name := s.providers[next]
	s.settings.ActiveProvider = name
	_ = s.settings.save()
	s.sendCore(map[string]any{"type": "set_provider", "name": name})
	s.logInfo(fmt.Sprintf("switching provider: %s", name))
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
		"  ctrl+r            set reasoning effort (per model)",
		"  ctrl+c            quit",
		"  esc               close modal / deny approval / drop queued · abort turn",
		"",
		"Scrolling the transcript",
		"  pgup / pgdn       scroll a page",
		"  ctrl+↑ / ctrl+↓   scroll a line",
		"  ctrl+home/end     jump to top / bottom",
		"  (scrolling up pauses auto-follow; sending a",
		"   message or reaching the bottom re-pins)",
		"",
		"Mouse & copy",
		"  click-drag selects/copies text (mouse off by default)",
		"  /settings → Mouse Wheel enables wheel scrolling",
		"  (hold Shift to select/copy while the mouse is on)",
		"",
		"While a turn is running (in-flight)",
		"  enter             queue a follow-up message",
		"  ctrl+enter        steer (interrupt + redirect the model)",
		"  esc               drop the queued message, or abort if none queued",
		"  /steer <msg>      steer (works on every terminal)",
		"",
		"Approval (when prompted)",
		"  y                 approve once",
		"  a                 approve & stop asking",
		"  n                 deny",
		"",
		"Slash commands",
		"  /login           log in / switch provider (OpenAI · Gemini · Anthropic)",
		"  /logout          log out of a provider",
		"  /model [N|substr] list or switch model",
		"  /approval <mode>  never | destructive | always",
		"  /reasoning        set reasoning effort (per model)",
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
		"  /vision          configure vision models & handoff target",
		"",
		"Settings persist to ~/.config/umans-harness/settings.json",
		"Config (core) persists to ~/.config/umans-harness/config.json",
		"",
		"Custom providers (OpenAI- & Anthropic-compatible endpoints)",
		"  Define named providers in the core config file's `providers` array:",
		"    { \"name\": \"anthropic\", \"kind\": \"anthropic\",",
		"      \"base_url\": \"https://api.anthropic.com/v1\",",
		"      \"api_key_env\": \"ANTHROPIC_API_KEY\" }",
		"    { \"name\": \"local\", \"kind\": \"openai\", \"base_url\": \"http://localhost:11434/v1\" }",
		"  Select one at startup with `--provider <name>` or UMANS_ACTIVE_PROVIDER.",
		"  Switch at runtime: /settings -> Provider (cycles + re-discovers models).",
		"  Each provider keeps its own key (/key stores per-provider).",
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
	w := s.width
	h := s.height
	// Safety net: never let the modal exceed the terminal. If a body still
	// comes out taller than the window (e.g. a non-scrolling modal on a very
	// short terminal), clip it to the terminal height so lipgloss.Place can't
	// overflow the canvas and scroll the terminal.
	if bh := lipgloss.Height(box); bh > h && h > 0 {
		ls := strings.Split(box, "\n")
		if h <= len(ls) {
			box = strings.Join(ls[:h], "\n")
		}
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
	case modalProviders:
		if s.modal.editing {
			return s.renderLoginKeyBox()
		}
		return s.renderListModal("Log in / switch provider", s.providerItems(), true)
	case modalLogout:
		return s.renderListModal("Log out", s.logoutItems(), true)
	case modalVision:
		return s.renderListModal("Vision Models", s.visionItems(), true)
	case modalSettings:
		return s.renderSettingsModal()
	case modalHelp:
		return s.renderHelpModal()
	}
	return ""
}

// fitListRow builds a single-line list row — marker + label + desc — that
// fits width visible columns, truncating the label first (it is the least
// essential) so the description (e.g. "12 msgs · 2h ago") is kept whole. The
// marker is already styled; markerW is its visible width.
func fitListRow(marker, label, desc string, markerW, width int) string {
	budget := width - markerW
	if budget < 0 {
		budget = 0
	}
	if d := len([]rune(desc)); d > 0 {
		if 2+d <= budget {
			label = truncateFit(label, budget-2-d)
		} else {
			// desc alone fills the row: drop the label, truncate the desc
			label = ""
			desc = truncateFit(desc, budget)
		}
	} else {
		label = truncateFit(label, budget)
	}
	row := marker + baseStyle.Render(label)
	switch {
	case label != "" && desc != "":
		row += "  " + dimStyle.Render(desc)
	case desc != "":
		row += dimStyle.Render(desc)
	}
	return row
}

// modalWidth returns a responsive modal width: as wide as the terminal
// allows (minus margins) up to cap, floored at 28. Replaces the old fixed
// 52/60 so longer content — session names especially — stays visible instead
// of being truncated.
func (s *session) modalWidth(cap int) int {
	w := s.width - 4
	if w > cap {
		w = cap
	}
	if w < 28 {
		w = 28
	}
	return w
}

func (s *session) renderListModal(title string, items []listItem, showFilter bool) string {
	w := s.modalWidth(110)
	idx := filterList(items, s.modal.filter)
	n := len(idx)

	// visible window: cap so long lists scroll instead of overflowing.
	// Overhead (title + filter + separator + "(N more)" + blank + footer +
	// the 2 border rows) is 8 lines, so maxVisible = height-9 leaves one line of
	// headroom. Floor at 1 (not 4) so short terminals still fit without overflow.
	maxVisible := s.height - 9
	if maxVisible < 1 {
		maxVisible = 1
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
	// truncStyle caps a line to a single line of rowW visible columns. Without
	// it, a long label+desc wraps inside modalBox's fixed width and each row
	// spans 2+ physical lines, so the modal grows past maxVisible and overflows
	// the terminal.
	truncStyle := lipgloss.NewStyle().MaxWidth(rowW)

	var lines []string
	lines = append(lines, accentStyle.Render("◆ "+title))
	if showFilter {
		fq := s.modal.filter
		if fq == "" {
			fq = dimStyle.Render("type to filter…")
		}
		lines = append(lines, truncStyle.Render(inputPromptStyle.Render("⟩ ")+mutedStyle.Render(fq)))
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
		// Fit marker + label + desc into one line of rowW columns, truncating the
		// label first so the description (msg count · time) is kept whole.
		row := fitListRow(marker, items[abs].label, items[abs].desc, 2, rowW)
		row = truncStyle.Render(row) // safety: guarantee a single line ≤ rowW
		if vi == s.modal.cursor {
			row = hiStyle.Render(row) // full-width highlight bar (pads to rowW)
		}
		lines = append(lines, row)
	}
	if n > maxVisible {
		lines = append(lines, dimStyle.Render(fmt.Sprintf("  (%d more · ↑↓ scroll)", n-maxVisible)))
	}
	lines = append(lines, "")
	footer := "  ↑↓ navigate · enter select · esc close"
	if s.modal.kind == modalPlugins {
		footer = "  ↑↓ navigate · enter toggle enable/disable · esc close"
	}
	if s.modal.kind == modalVision {
		footer = "  ↑↓ navigate · space toggle vision · enter set target · esc close"
	}
	lines = append(lines, truncStyle.Render(dimStyle.Render(footer)))
	body := strings.Join(lines, "\n")
	return modalBox(w, body)
}

func (s *session) renderSettingsModal() string {
	w := s.modalWidth(72)
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
	w := s.modalWidth(80)
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
