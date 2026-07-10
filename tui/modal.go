package main

import (
	"encoding/json"
	"fmt"
	"strings"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

// ---------------------------------------------------------------------------
// Modal framework
//
// An overlay layer that renders on top of the viewport and intercepts keys.
// List modals share a filterable-list core; value-edit modals capture a single
// free-form field (API key, timeouts). /settings is a hub that opens the
// dedicated modal (or slash command) for each preference.
// ---------------------------------------------------------------------------

type modalKind int

const (
	modalNone modalKind = iota
	modalCommand
	modalModels
	modalSettings // hub of setting commands → dedicated modals
	modalHelp
	modalTheme
	modalSessions
	modalPlugins
	modalReasoning
	modalVision
	modalProviders
	modalLogout
	modalKeybinds
	modalOauthCode
	modalContext
	modalUsage // provider plan / rate-limit usage (/usage)
	modalApproval
	modalSandbox
	modalAutoCompact
	modalNoNetwork
	modalMouseWheel
	modalValueEdit // free-form edit (api_key, timeouts, remember, attach, run, …)
	modalMemory    // pick a memory to forget
	modalGoal      // multi-field /goal form (goal, concurrency, models, providers)
	modalGoalPlan  // plan-ready review (approve / revise / cancel)
)

// goalDraft is the multi-field form state for modalGoal.
type goalDraft struct {
	goal               string
	concurrency        int
	maxTasks           int
	allowedModels      map[string]bool // model id → selected; empty map = unrestricted
	allowedProviders   map[string]bool // provider name → selected; empty = unrestricted
	reviewBeforeDeploy bool            // auto_deploy = !reviewBeforeDeploy
	// Advanced section
	advanced         bool
	plannerModel     string         // empty = default (orchestrator)
	workerModel      string
	reviewerModel    string
	modelConcurrency map[string]int // model id → max concurrent (capped by concurrency)
	field            int            // focused field id (goalField*)
	listCursor       int            // cursor within models/providers/model-conc lists
	editing          bool           // free-text capture for goal field
}

const (
	goalFieldGoal = iota
	goalFieldConcurrency
	goalFieldMaxTasks
	goalFieldProviders
	goalFieldModels
	goalFieldReview
	goalFieldAdvanced // checkbox — expand advanced role/model limits
	goalFieldPlanner
	goalFieldWorker
	goalFieldReviewer
	goalFieldModelConc // per-model concurrency list
	goalFieldStart
)

// goalVisibleFields returns the field order for the current advanced state.
func goalVisibleFields(advanced bool) []int {
	base := []int{
		goalFieldGoal,
		goalFieldConcurrency,
		goalFieldMaxTasks,
		goalFieldProviders,
		goalFieldModels,
		goalFieldReview,
		goalFieldAdvanced,
	}
	if advanced {
		base = append(base,
			goalFieldPlanner,
			goalFieldWorker,
			goalFieldReviewer,
			goalFieldModelConc,
		)
	}
	return append(base, goalFieldStart)
}

func goalFieldIndex(fields []int, field int) int {
	for i, f := range fields {
		if f == field {
			return i
		}
	}
	return 0
}

func goalNextField(fields []int, field, delta int) int {
	i := goalFieldIndex(fields, field)
	n := len(fields)
	if n == 0 {
		return field
	}
	i = (i + delta%n + n) % n
	return fields[i]
}

// goalStateSnap is a lightweight view of the core's goal_state event.
type goalStateSnap struct {
	ID        string
	Goal      string
	Phase     string
	Error     string
	AutoDeploy bool
	Prompts   []goalPromptSnap
	Version   uint64
}

type goalPromptSnap struct {
	StepID string
	Agent  string
	Title  string
	Status string
	Summary string
}

type goalPlanSnap struct {
	Summary    string
	Steps      []map[string]any
	Risks      []string
	Validation []string
}

// Value-edit targets for modalValueEdit (stored in modal.editTarget).
const (
	editTargetAPIKey           = "api_key"
	editTargetBashTimeout      = "bash_timeout"
	editTargetIdleTimeout      = "idle_timeout"
	editTargetMaxSessionTokens = "max_session_tokens"
	editTargetRemember         = "remember"
	editTargetAttach           = "attach"
	editTargetPluginInstall    = "plugin_install"
	editTargetSteer            = "steer"
	editTargetRun              = "run"
	editTargetParallel         = "parallel"
	editTargetChain            = "chain"
	editTargetCompact          = "compact"
)

// Plugin picker modes (session.pluginPickerMode).
const (
	pluginModeToggle = "toggle"
	pluginModeRemove = "remove"
)

type modal struct {
	kind       modalKind
	filter     string // typed filter (list modals)
	cursor     int    // selected index in the filtered list
	scroll     int    // help modal vertical scroll
	fieldIdx   int    // legacy field index (unused by hub; kept for edit buffer routing)
	editing    bool   // value-edit / login key capture
	editBuf    textinput.Model
	editTarget string // which setting modalValueEdit is editing
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

// openSettings opens the settings hub — a list of dedicated setting commands.
// Each entry dispatches to its own modal (or slash command) rather than the
// old multi-field settings editor.
func (s *session) openSettings() {
	s.modal = newModal()
	s.modal.kind = modalSettings
	s.modal.cursor = 0
}

// openApprovalPicker lists never / destructive / always for the safety gate.
func (s *session) openApprovalPicker() {
	s.modal = newModal()
	s.modal.kind = modalApproval
	s.modal.cursor = 0
	modes := []string{"never", "destructive", "always"}
	cur := s.approvalMode()
	for i, m := range modes {
		if m == cur {
			s.modal.cursor = i
			break
		}
	}
}

// openSandboxPicker lists sandbox modes (none | firejail).
func (s *session) openSandboxPicker() {
	s.modal = newModal()
	s.modal.kind = modalSandbox
	s.modal.cursor = 0
	modes := []string{"none", "firejail"}
	for i, m := range modes {
		if m == s.settings.Sandbox {
			s.modal.cursor = i
			break
		}
	}
}

// openAutoCompactPicker toggles auto context compaction via a two-item list.
func (s *session) openAutoCompactPicker() {
	s.modal = newModal()
	s.modal.kind = modalAutoCompact
	if s.coreAutoCompact {
		s.modal.cursor = 0 // on
	} else {
		s.modal.cursor = 1 // off
	}
}

// openNoNetworkPicker toggles the no-network sandbox flag.
func (s *session) openNoNetworkPicker() {
	s.modal = newModal()
	s.modal.kind = modalNoNetwork
	if s.settings.NoNetwork {
		s.modal.cursor = 0
	} else {
		s.modal.cursor = 1
	}
}

// openMouseWheelPicker toggles mouse-wheel scrolling.
func (s *session) openMouseWheelPicker() {
	s.modal = newModal()
	s.modal.kind = modalMouseWheel
	if s.settings.MouseWheel {
		s.modal.cursor = 0
	} else {
		s.modal.cursor = 1
	}
}

// openValueEditModal opens a free-form edit box for a single numeric/text setting.
func (s *session) openValueEditModal(target, title, placeholder, initial string) {
	s.modal = newModal()
	s.modal.kind = modalValueEdit
	s.modal.editing = true
	s.modal.editTarget = target
	s.modal.filter = title // reuse filter as the modal title for value edit
	ti := textinput.New()
	ti.Prompt = ""
	ti.Placeholder = placeholder
	ti.SetValue(initial)
	ti.CursorEnd()
	ti.Focus()
	s.modal.editBuf = ti
}

func (s *session) openAPIKeyModal() {
	s.openValueEditModal(editTargetAPIKey, "API Key", "sk-... (active provider)", "")
}

func (s *session) openBashTimeoutModal() {
	s.openValueEditModal(editTargetBashTimeout, "Bash Timeout (seconds)",
		fmt.Sprintf("%d", s.coreBashTimeout), fmt.Sprintf("%d", s.coreBashTimeout))
}

func (s *session) openIdleTimeoutModal() {
	s.openValueEditModal(editTargetIdleTimeout, "Idle Timeout (seconds)",
		fmt.Sprintf("%d", s.settings.IdleTimeout), fmt.Sprintf("%d", s.settings.IdleTimeout))
}

func (s *session) openMaxSessionTokensModal() {
	s.openValueEditModal(editTargetMaxSessionTokens, "Max Session Tokens (0=unlimited)",
		fmt.Sprintf("%d", s.settings.MaxSessionTokens), fmt.Sprintf("%d", s.settings.MaxSessionTokens))
}

// openRememberModal collects a durable memory note without requiring
// `/remember <text>` on the command line.
func (s *session) openRememberModal() {
	s.openValueEditModal(editTargetRemember, "Remember", "durable note for future sessions", "")
}

// openAttachModal collects an image path for vision (optional prompt uses the
// current composer text, same as `/attach <path>` with no trailing prompt).
func (s *session) openAttachModal() {
	s.openValueEditModal(editTargetAttach, "Attach Image", "/path/to/image.png", "")
}

// openPluginInstallModal collects a filesystem path to a plugin directory.
func (s *session) openPluginInstallModal() {
	s.openValueEditModal(editTargetPluginInstall, "Install Plugin", "/path/to/plugin-dir", "")
}

// openSteerModal collects a mid-turn steer message.
func (s *session) openSteerModal() {
	s.openValueEditModal(editTargetSteer, "Steer", "mid-turn instruction for the agent", "")
}

// openRunModal / openParallelModal / openChainModal collect the free-form
// remainder of a subagent slash command (agent + task syntax).
func (s *session) openRunModal() {
	s.openValueEditModal(editTargetRun, "Run Subagent", `agent "task description"`, "")
}

func (s *session) openParallelModal() {
	s.openValueEditModal(editTargetParallel, "Parallel Subagents",
		`a1 "task1" | a2 "task2"`, "")
}

func (s *session) openChainModal() {
	s.openValueEditModal(editTargetChain, "Chain Subagents",
		`a1 "task1" -> a2 "task2"`, "")
}

// openCompactModal optionally collects compaction instructions; empty Enter
// forces a default compaction (same as bare `/compact`).
func (s *session) openCompactModal() {
	s.openValueEditModal(editTargetCompact, "Compact Context",
		"optional: what to preserve (blank = default)", "")
}

// openGoalModal opens the multi-field goal form. prefill seeds the goal text
// when the user typed `/goal fix auth` (still confirm concurrency/models).
func (s *session) openGoalModal(prefill string) {
	s.modal = newModal()
	s.modal.kind = modalGoal
	d := goalDraft{
		goal:               strings.TrimSpace(prefill),
		concurrency:        4,
		maxTasks:           8,
		allowedModels:      map[string]bool{},
		allowedProviders:   map[string]bool{},
		reviewBeforeDeploy: false,
		advanced:           false,
		modelConcurrency:   map[string]int{},
		field:              goalFieldGoal,
	}
	if d.goal == "" {
		d.editing = true
		ti := textinput.New()
		ti.Prompt = ""
		ti.Placeholder = "describe the goal to plan & deploy…"
		ti.Focus()
		s.modal.editBuf = ti
		s.modal.editing = true
	}
	s.goalDraft = d
}

// openGoalPlanReview shows approve / revise / cancel for a plan_ready goal.
func (s *session) openGoalPlanReview() {
	s.modal = newModal()
	s.modal.kind = modalGoalPlan
	s.modal.cursor = 0
}

// openMemoryPicker shows saved memories so the user can forget one by Enter.
// Call after memoryList has been populated from a list_memory core event.
func (s *session) openMemoryPicker() {
	s.modal = newModal()
	s.modal.kind = modalMemory
	s.modal.cursor = 0
}

// requestMemoryPicker asks the core for the memory list and opens the picker
// once the response arrives (see memory_list event handling).
func (s *session) requestMemoryPicker() {
	s.pendingMemoryPicker = true
	s.sendCore(map[string]any{"type": "list_memory"})
	s.logInfo("loading memories…")
}

// requestPluginPicker asks the core for plugins and opens the plugin modal
// (toggle or remove mode) when plugins_list arrives.
func (s *session) requestPluginPicker(mode string) {
	if mode == "" {
		mode = pluginModeToggle
	}
	s.pluginPickerMode = mode
	s.sendCore(map[string]any{"type": "list_plugins"})
	if mode == pluginModeRemove {
		s.logInfo("loading plugins (enter to uninstall)…")
	} else {
		s.logInfo("loading plugins…")
	}
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
			if p.SupportsOauth && p.EnvVar == "" {
				// OAuth-only (e.g. xAI SuperGrok) — no API key override.
				if p.ID == s.activeProvider {
					desc = "logged in · active · enter to switch · " + desc
				} else {
					desc = "logged in · enter to switch · " + desc
				}
			} else if p.ID == s.activeProvider {
				desc = "logged in · active · enter to override key (empty = switch) · " + desc
			} else {
				desc = "logged in · enter to override key (empty = switch) · " + desc
			}
		case p.HasKey:
			label = "▸ " + p.Label
			if p.SupportsOauth && p.EnvVar == "" {
				desc = "OAuth credentials on disk · enter to re-login · " + desc
			} else {
				desc = "ready (key in " + p.EnvVar + ") · enter to log in · " + desc
			}
		default:
			label = "▸ " + p.Label
			if p.SupportsOauth && p.EnvVar == "" {
				desc = "enter to log in via OAuth (SuperGrok / X Premium+) · " + desc
			} else if p.SupportsOauth {
				desc = "enter to log in via OAuth (browser) · or set " + p.EnvVar + " · " + desc
			} else {
				desc = "enter key to log in · needs " + p.EnvVar + " · " + desc
			}
		}
		items = append(items, listItem{label: label, desc: desc, meta: p.ID, meta2: "preset"})
	}
	// Configured providers not covered by a preset (e.g. custom/local).
	for _, name := range s.providers {
		if s.presetByID(name) != nil || isPresetCompanion(name) {
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
		if s.presetByID(name) != nil || isPresetCompanion(name) {
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

// isPresetCompanion reports whether a configured provider name is a non-primary
// companion of a first-party preset (e.g. "opencode-go-anthropic" backs the
// "opencode-go" preset). OpenCode Go is one subscription served over two wire
// protocols, so the core creates two provider configs from one preset; these
// companions are hidden from the login/logout pickers so the user only sees the
// single preset entry, while the core still creates/removes both together.
func isPresetCompanion(name string) bool {
	return name == "opencode-go-anthropic"
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
// target) to the core, which writes .catalyst-code/vision.json and echoes a
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
func (s *session) handleVisionKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	items := s.visionItems()
	idx := filterList(items, s.modal.filter)
	n := len(idx)
	switch {
	case msg.String() == "up" || s.kbAny(msg, "nav_up", "nav_up_alt"):
		if n > 0 {
			s.modal.cursor = (s.modal.cursor - 1 + n) % n
		}
	case msg.String() == "down" || s.kbAny(msg, "nav_down", "nav_down_alt"):
		if n > 0 {
			s.modal.cursor = (s.modal.cursor + 1) % n
		}
	case msg.String() == " " || s.kb(msg, "vision_toggle"):
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
	case msg.String() == "enter" || s.kb(msg, "select"):
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
	case msg.String() == "backspace":
		if len(s.modal.filter) > 0 {
			r := []rune(s.modal.filter)
			s.modal.filter = string(r[:len(r)-1])
			s.modal.cursor = 0
		}
	case s.kb(msg, "filter_clear"):
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
	s.pluginPickerMode = ""
}

// ---------------------------------------------------------------------------
// Filtered-list computation
// ---------------------------------------------------------------------------

func (s *session) commandItems() []listItem {
	items := []listItem{
		{label: "/login", desc: "log in / switch provider (OpenAI · Gemini · Anthropic)"},
		{label: "/logout", desc: "log out of a provider"},
		{label: "/oauth-code", desc: "paste OAuth code (SSH/headless Google login)"},
		{label: "/key", desc: "set API key for active provider"},
		{label: "/model", desc: "switch model"},
		{label: "/approval", desc: "never · destructive · always"},
		{label: "/reasoning", desc: "set reasoning effort (per model)"},
		{label: "/theme", desc: "switch colour theme"},
		{label: "/bash-timeout", desc: "bash tool timeout (seconds)"},
		{label: "/auto-compact", desc: "auto context compaction on/off"},
		{label: "/sandbox", desc: "sandbox mode (none · firejail)"},
		{label: "/no-network", desc: "block network in sandbox on/off"},
		{label: "/mouse-wheel", desc: "mouse-wheel scrolling on/off"},
		{label: "/idle-timeout", desc: "idle timeout (seconds)"},
		{label: "/max-session-tokens", desc: "max session tokens (0=unlimited)"},
		{label: "/reset", desc: "wipe conversation + session file"},
		{label: "/clear", desc: "clear view (keep session file)"},
		{label: "/undo", desc: "drop last turn"},
		{label: "/compact", desc: "force compaction (modal for optional instructions)"},
		{label: "/sessions", desc: "open session picker"},
		{label: "/new", desc: "start a fresh session file"},
		{label: "/stats", desc: "token + turn totals"},
		{label: "/context", desc: "token-usage breakdown (top consumers)"},
		{label: "/usage", desc: "provider plan limits (5h · weekly · …)"},
		{label: "/abort", desc: "stop running turn (or Esc)"},
		{label: "/steer", desc: "steer an in-flight turn (modal)"},
		{label: "/settings", desc: "settings hub (dedicated modals per option)"},
		{label: "/keybinds", desc: "view & customize keybindings"},
		{label: "/help", desc: "keybindings & commands"},
		{label: "/copy", desc: "copy last assistant reply"},
		{label: "/attach", desc: "attach an image (vision) — path modal"},
		{label: "/vision", desc: "configure vision models & handoff target"},
		{label: "/plugin-install", desc: "install a plugin from directory (path modal)"},
		{label: "/plugin-config", desc: "list plugins · enter to enable/disable"},
		{label: "/plugin-remove", desc: "uninstall a plugin (picker)"},
		{label: "/goal", desc: "goal mode — plan & deploy subagents (modal)"},
		{label: "/run", desc: "delegate to a subagent (single) — modal"},
		{label: "/parallel", desc: "run subagents in parallel — modal"},
		{label: "/chain", desc: "run a subagent chain — modal"},
		{label: "/subagents", desc: "list available subagents"},
		{label: "/cancel-goal", desc: "cancel active goal mode"},
		{label: "/subagents-doctor", desc: "subagent setup diagnostics"},
		{label: "/subagents-status", desc: "show active subagent runs"},
		{label: "/remember", desc: "save a memory note (modal)"},
		{label: "/memory", desc: "list / forget saved memories (picker)"},
		{label: "/forget", desc: "forget a memory (picker)"},
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

// settingsHubItems is the /settings list — one entry per preference, each
// opening its dedicated modal (or slash command). Values shown in the desc
// so the hub is a live overview, not a second settings editor.
func (s *session) settingsHubItems() []listItem {
	return []listItem{
		{label: "/login", desc: "provider · " + s.providerFieldLabel()},
		{label: "/key", desc: "API key for active provider"},
		{label: "/approval", desc: "safety gate · " + s.approvalMode()},
		{label: "/reasoning", desc: "effort · " + s.settings.ReasoningEffort},
		{label: "/theme", desc: "colour · " + activeTheme.name},
		{label: "/bash-timeout", desc: fmt.Sprintf("%ds", s.coreBashTimeout)},
		{label: "/auto-compact", desc: boolStr(s.coreAutoCompact)},
		{label: "/sandbox", desc: s.settings.Sandbox},
		{label: "/no-network", desc: boolStr(s.settings.NoNetwork) + " (next launch)"},
		{label: "/mouse-wheel", desc: boolStr(s.settings.MouseWheel)},
		{label: "/idle-timeout", desc: fmt.Sprintf("%ds (next launch)", s.settings.IdleTimeout)},
		{label: "/max-session-tokens", desc: fmt.Sprintf("%d (next launch)", s.settings.MaxSessionTokens)},
		{label: "/keybinds", desc: "view & customize keybindings"},
	}
}

func (s *session) approvalItems() []listItem {
	modes := []struct {
		mode, desc string
	}{
		{"never", "auto-approve all tools"},
		{"destructive", "prompt for write / bash / destructive only"},
		{"always", "prompt for every tool call"},
	}
	cur := s.approvalMode()
	items := make([]listItem, len(modes))
	for i, m := range modes {
		desc := m.desc
		if m.mode == cur {
			desc = "current · " + desc
		}
		items[i] = listItem{label: m.mode, desc: desc}
	}
	return items
}

func (s *session) sandboxItems() []listItem {
	modes := []struct {
		mode, desc string
	}{
		{"none", "no sandbox (applies on next launch)"},
		{"firejail", "firejail sandbox (applies on next launch)"},
	}
	items := make([]listItem, len(modes))
	for i, m := range modes {
		desc := m.desc
		if m.mode == s.settings.Sandbox {
			desc = "current · " + desc
		}
		items[i] = listItem{label: m.mode, desc: desc}
	}
	return items
}

// toggleItems builds a two-option on/off list with "current" marked.
func toggleItems(on bool, onDesc, offDesc string) []listItem {
	onItem := listItem{label: "on", desc: onDesc}
	offItem := listItem{label: "off", desc: offDesc}
	if on {
		onItem.desc = "current · " + onDesc
	} else {
		offItem.desc = "current · " + offDesc
	}
	return []listItem{onItem, offItem}
}

func (s *session) autoCompactItems() []listItem {
	return toggleItems(s.coreAutoCompact,
		"compact context automatically when full",
		"never auto-compact")
}

func (s *session) noNetworkItems() []listItem {
	return toggleItems(s.settings.NoNetwork,
		"block network in sandbox (next launch)",
		"allow network (next launch)")
}

func (s *session) mouseWheelItems() []listItem {
	return toggleItems(s.settings.MouseWheel,
		"wheel scrolls transcript (Shift+drag to select)",
		"native click-drag select/copy")
}

func (s *session) pluginItems() []listItem {
	removeMode := s.pluginPickerMode == pluginModeRemove
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
		var action string
		if removeMode {
			label += " · uninstall"
			action = "uninstall"
		} else if enabled == "false" {
			label += " (disabled)"
			action = "enable"
		} else {
			label += " (enabled)"
			action = "disable"
		}
		hint := "enter to " + action
		if desc != "" {
			hint = desc + " · " + hint
		}
		items = append(items, listItem{label: label, desc: hint})
	}
	if len(items) == 0 {
		items = append(items, listItem{label: "(no plugins installed)", desc: "use /plugin-install to add one"})
	}
	return items
}

// memoryItems builds the pick-to-forget list for modalMemory.
func (s *session) memoryItems() []listItem {
	items := make([]listItem, 0, len(s.memoryList))
	for _, e := range s.memoryList {
		id := e.ID
		if id == "" {
			id = "?"
		}
		label := truncateRunes(e.Text, 80)
		if label == "" {
			label = "(empty)"
		}
		desc := "id " + id + " · enter to forget"
		if len(e.Tags) > 0 {
			desc = "[" + strings.Join(e.Tags, ",") + "] · " + desc
		}
		items = append(items, listItem{label: label, desc: desc, meta: id})
	}
	if len(items) == 0 {
		items = append(items, listItem{label: "(no memories)", desc: "use /remember to add one"})
	}
	return items
}

// removePlugin uninstalls the plugin at store index idx and drops it from the
// cached store so the picker re-renders immediately.
func (s *session) removePlugin(idx int) {
	if idx < 0 || idx >= len(sPluginStore) {
		return
	}
	var m map[string]any
	if json.Unmarshal(sPluginStore[idx], &m) != nil {
		return
	}
	name, _ := m["name"].(string)
	if name == "" {
		return
	}
	s.sendCore(map[string]any{"type": "remove_plugin", "name": name})
	s.logInfo("removing plugin: " + name)
	// Drop from cache so the row disappears without a list_plugins round-trip.
	sPluginStore = append(sPluginStore[:idx], sPluginStore[idx+1:]...)
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

func (s *session) handleModalKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	// The keybinds modal has its own capture mode (editing flag) and navigation;
	// route to it first so capture works even while editing is active.
	if s.modal.kind == modalKeybinds {
		return s.handleKeybindsKey(msg)
	}
	// Goal form owns its own text capture (goalDraft.editing) — do not route
	// through the single-field value-edit commit path.
	if s.modal.kind == modalGoal {
		return s.handleGoalKey(msg)
	}
	if s.modal.kind == modalGoalPlan {
		return s.handleGoalPlanKey(msg)
	}
	// While editing a value field (settings value-edit, login key, oauth code),
	// route keys to the edit buffer.
	if s.modal.editing {
		return s.handleSettingsEditKey(msg)
	}

	if s.kbAny(msg, "close", "quit") {
		if s.modal.kind != modalNone {
			s.closeModal()
			return s, nil
		}
	}

	switch s.modal.kind {
	case modalCommand, modalModels, modalTheme, modalSessions, modalPlugins, modalReasoning,
		modalProviders, modalLogout, modalSettings, modalApproval, modalSandbox,
		modalAutoCompact, modalNoNetwork, modalMouseWheel, modalMemory:
		return s.handleListKey(msg)
	case modalVision:
		return s.handleVisionKey(msg)
	case modalHelp:
		return s.handleHelpKey(msg)
	case modalContext, modalUsage:
		// Display-only modal: enter or esc dismisses it.
		if msg.String() == "enter" || s.kb(msg, "select") || s.kbAny(msg, "close", "quit") {
			s.closeModal()
		}
		return s, nil
	}
	return s, nil
}

// handleGoalKey drives the multi-field /goal form.
func (s *session) handleGoalKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	d := &s.goalDraft
	key := msg.String()
	fields := goalVisibleFields(d.advanced)

	// Ctrl+Enter submits from any field.
	if key == "ctrl+enter" || key == "ctrl+j" {
		return s, s.submitGoalModal()
	}

	// While editing the goal text field, route to textinput.
	if d.editing && d.field == goalFieldGoal {
		if key == "esc" {
			d.goal = s.modal.editBuf.Value()
			d.editing = false
			s.modal.editing = false
			return s, nil
		}
		if key == "enter" {
			d.goal = strings.TrimSpace(s.modal.editBuf.Value())
			d.editing = false
			s.modal.editing = false
			d.field = goalFieldConcurrency
			return s, nil
		}
		var cmd tea.Cmd
		s.modal.editBuf, cmd = s.modal.editBuf.Update(msg)
		return s, cmd
	}

	switch {
	case key == "up" || s.kbAny(msg, "nav_up", "nav_up_alt"):
		if d.field == goalFieldProviders || d.field == goalFieldModels || d.field == goalFieldModelConc {
			if d.listCursor > 0 {
				d.listCursor--
				return s, nil
			}
		}
		d.field = goalNextField(fields, d.field, -1)
		d.listCursor = 0
	case key == "down" || key == "tab" || s.kbAny(msg, "nav_down", "nav_down_alt"):
		if d.field == goalFieldProviders || d.field == goalFieldModels || d.field == goalFieldModelConc {
			n := s.goalListLen(d.field)
			if d.listCursor+1 < n {
				d.listCursor++
				return s, nil
			}
		}
		d.field = goalNextField(fields, d.field, 1)
		d.listCursor = 0
	case key == "left":
		switch d.field {
		case goalFieldConcurrency:
			if d.concurrency > 1 {
				d.concurrency--
			}
		case goalFieldMaxTasks:
			if d.maxTasks > 1 {
				d.maxTasks--
			}
		case goalFieldPlanner:
			d.plannerModel = s.cycleGoalRoleModel(d.plannerModel, -1)
		case goalFieldWorker:
			d.workerModel = s.cycleGoalRoleModel(d.workerModel, -1)
		case goalFieldReviewer:
			d.reviewerModel = s.cycleGoalRoleModel(d.reviewerModel, -1)
		case goalFieldModelConc:
			s.adjustGoalModelConc(d.listCursor, -1)
		}
	case key == "right":
		switch d.field {
		case goalFieldConcurrency:
			if d.concurrency < d.maxTasks {
				d.concurrency++
			}
		case goalFieldMaxTasks:
			if d.maxTasks < 64 {
				d.maxTasks++
			}
		case goalFieldPlanner:
			d.plannerModel = s.cycleGoalRoleModel(d.plannerModel, 1)
		case goalFieldWorker:
			d.workerModel = s.cycleGoalRoleModel(d.workerModel, 1)
		case goalFieldReviewer:
			d.reviewerModel = s.cycleGoalRoleModel(d.reviewerModel, 1)
		case goalFieldModelConc:
			s.adjustGoalModelConc(d.listCursor, 1)
		}
	case key == " " || key == "space":
		switch d.field {
		case goalFieldProviders:
			s.toggleGoalProvider(d.listCursor)
		case goalFieldModels:
			s.toggleGoalModel(d.listCursor)
		case goalFieldReview:
			d.reviewBeforeDeploy = !d.reviewBeforeDeploy
		case goalFieldAdvanced:
			d.advanced = !d.advanced
			if !d.advanced {
				// Leave advanced fields; clamp focus to visible set.
				d.field = goalFieldAdvanced
			}
		}
	case key == "enter" || s.kb(msg, "select"):
		switch d.field {
		case goalFieldGoal:
			d.editing = true
			s.modal.editing = true
			ti := textinput.New()
			ti.Prompt = ""
			ti.Placeholder = "describe the goal to plan & deploy…"
			ti.SetValue(d.goal)
			ti.CursorEnd()
			ti.Focus()
			s.modal.editBuf = ti
		case goalFieldReview:
			d.reviewBeforeDeploy = !d.reviewBeforeDeploy
		case goalFieldAdvanced:
			d.advanced = !d.advanced
		case goalFieldProviders:
			s.toggleGoalProvider(d.listCursor)
		case goalFieldModels:
			s.toggleGoalModel(d.listCursor)
		case goalFieldPlanner:
			d.plannerModel = s.cycleGoalRoleModel(d.plannerModel, 1)
		case goalFieldWorker:
			d.workerModel = s.cycleGoalRoleModel(d.workerModel, 1)
		case goalFieldReviewer:
			d.reviewerModel = s.cycleGoalRoleModel(d.reviewerModel, 1)
		case goalFieldStart:
			return s, s.submitGoalModal()
		case goalFieldConcurrency:
			d.concurrency++
			if d.concurrency > d.maxTasks {
				d.concurrency = 1
			}
		case goalFieldMaxTasks:
			d.maxTasks++
			if d.maxTasks > 64 {
				d.maxTasks = 1
			}
			if d.concurrency > d.maxTasks {
				d.concurrency = d.maxTasks
			}
		}
	case key == "esc":
		s.closeModal()
	}
	return s, nil
}

func (s *session) goalListLen(field int) int {
	switch field {
	case goalFieldProviders:
		return len(s.goalProviderOptions())
	case goalFieldModels:
		return len(s.goalModelOptions())
	case goalFieldModelConc:
		return len(s.goalModelConcOptions())
	}
	return 0
}

// cycleGoalRoleModel walks models including "" (default). delta ±1.
func (s *session) cycleGoalRoleModel(cur string, delta int) string {
	opts := s.goalModelOptions()
	// Leading empty = default (parent / allowlist).
	all := append([]string{""}, opts...)
	idx := 0
	for i, m := range all {
		if m == cur {
			idx = i
			break
		}
	}
	n := len(all)
	if n == 0 {
		return ""
	}
	idx = (idx + delta%n + n) % n
	return all[idx]
}

// goalModelConcOptions lists models that can have per-model concurrency:
// selected allowlist models if any, else all models (filtered by provider).
func (s *session) goalModelConcOptions() []string {
	opts := s.goalModelOptions()
	if len(s.goalDraft.allowedModels) == 0 {
		return opts
	}
	var out []string
	for _, m := range opts {
		if s.goalDraft.allowedModels[m] {
			out = append(out, m)
		}
	}
	// Always include role pins so their limits can be set.
	for _, role := range []string{s.goalDraft.plannerModel, s.goalDraft.workerModel, s.goalDraft.reviewerModel} {
		if role == "" {
			continue
		}
		found := false
		for _, m := range out {
			if m == role {
				found = true
				break
			}
		}
		if !found {
			out = append(out, role)
		}
	}
	return out
}

func (s *session) adjustGoalModelConc(idx, delta int) {
	opts := s.goalModelConcOptions()
	if idx < 0 || idx >= len(opts) {
		return
	}
	if s.goalDraft.modelConcurrency == nil {
		s.goalDraft.modelConcurrency = map[string]int{}
	}
	id := opts[idx]
	cur := s.goalDraft.modelConcurrency[id]
	if cur == 0 {
		cur = s.goalDraft.concurrency
	}
	cur += delta
	if cur < 1 {
		cur = 1
	}
	if cur > s.goalDraft.concurrency {
		cur = s.goalDraft.concurrency
	}
	// If equal to global, drop the override to keep payload clean.
	if cur == s.goalDraft.concurrency {
		delete(s.goalDraft.modelConcurrency, id)
	} else {
		s.goalDraft.modelConcurrency[id] = cur
	}
}

func (s *session) goalProviderOptions() []string {
	if len(s.providers) > 0 {
		return append([]string{}, s.providers...)
	}
	// Fall back to preset ids that are logged in.
	var out []string
	for _, p := range s.providerPresets {
		if p.LoggedIn || p.Configured {
			out = append(out, p.ID)
		}
	}
	return out
}

func (s *session) goalModelOptions() []string {
	provSel := s.goalDraft.allowedProviders
	restrictProv := len(provSel) > 0
	var out []string
	for _, m := range s.models {
		if restrictProv {
			// Count how many providers are selected; if none match this model's
			// provider, skip. Empty provider on model still included.
			if m.Provider != "" {
				if !provSel[m.Provider] {
					// also try case-insensitive
					ok := false
					for p, on := range provSel {
						if on && strings.EqualFold(p, m.Provider) {
							ok = true
							break
						}
					}
					if !ok {
						continue
					}
				}
			}
		}
		out = append(out, m.ID)
	}
	return out
}

func (s *session) toggleGoalProvider(idx int) {
	opts := s.goalProviderOptions()
	if idx < 0 || idx >= len(opts) {
		return
	}
	if s.goalDraft.allowedProviders == nil {
		s.goalDraft.allowedProviders = map[string]bool{}
	}
	id := opts[idx]
	if s.goalDraft.allowedProviders[id] {
		delete(s.goalDraft.allowedProviders, id)
	} else {
		s.goalDraft.allowedProviders[id] = true
	}
}

func (s *session) toggleGoalModel(idx int) {
	opts := s.goalModelOptions()
	if idx < 0 || idx >= len(opts) {
		return
	}
	if s.goalDraft.allowedModels == nil {
		s.goalDraft.allowedModels = map[string]bool{}
	}
	id := opts[idx]
	if s.goalDraft.allowedModels[id] {
		delete(s.goalDraft.allowedModels, id)
	} else {
		s.goalDraft.allowedModels[id] = true
	}
}

// submitGoalModal sends start_goal to the core.
func (s *session) submitGoalModal() tea.Cmd {
	d := s.goalDraft
	if d.editing {
		d.goal = strings.TrimSpace(s.modal.editBuf.Value())
	}
	goal := strings.TrimSpace(d.goal)
	if goal == "" {
		s.logError("goal text is required")
		return nil
	}
	if !s.authed {
		s.logError("not authenticated — run /login first")
		return nil
	}
	if len(s.models) == 0 {
		s.logError("no models loaded yet")
		return nil
	}
	model := s.models[s.modelIdx].ID
	var models []string
	for id, on := range d.allowedModels {
		if on {
			models = append(models, id)
		}
	}
	var providers []string
	for id, on := range d.allowedProviders {
		if on {
			providers = append(providers, id)
		}
	}
	concurrency := d.concurrency
	if concurrency < 1 {
		concurrency = 1
	}
	maxTasks := d.maxTasks
	if maxTasks < 1 {
		maxTasks = 8
	}
	if concurrency > maxTasks {
		concurrency = maxTasks
	}
	cmd := map[string]any{
		"type":             "start_goal",
		"goal":             goal,
		"concurrency":      concurrency,
		"max_tasks":        maxTasks,
		"auto_deploy":      !d.reviewBeforeDeploy,
		"model":            model,
		"reasoning_effort": s.settings.ReasoningEffort,
	}
	if len(models) > 0 {
		cmd["allowed_models"] = models
	}
	if len(providers) > 0 {
		cmd["allowed_providers"] = providers
	}
	if d.advanced {
		if d.plannerModel != "" {
			cmd["planner_model"] = d.plannerModel
		}
		if d.workerModel != "" {
			cmd["worker_model"] = d.workerModel
		}
		if d.reviewerModel != "" {
			cmd["reviewer_model"] = d.reviewerModel
		}
		if len(d.modelConcurrency) > 0 {
			mc := map[string]int{}
			for k, v := range d.modelConcurrency {
				if v > 0 && v < concurrency {
					mc[k] = v
				}
			}
			if len(mc) > 0 {
				cmd["model_concurrency"] = mc
			}
		}
	}
	s.closeModal()
	s.follow = true
	s.busy = true
	s.logUser(fmt.Sprintf("🎯 Goal: %s  ↳ /goal", goal))
	s.sendCore(cmd)
	return nil
}

func (s *session) handleGoalPlanKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	key := msg.String()
	switch {
	case key == "a" || key == "enter" || s.kb(msg, "select"):
		s.closeModal()
		s.sendCore(map[string]any{"type": "approve_goal_plan"})
		s.logInfo("approving goal plan…")
		s.busy = true
		return s, nil
	case key == "r":
		s.closeModal()
		// Open a value-edit for revise feedback.
		s.openValueEditModal("goal_revise", "Revise Goal Plan", "what should change?", "")
		return s, nil
	case key == "q" || key == "c" || key == "esc":
		s.closeModal()
		s.sendCore(map[string]any{"type": "cancel_goal"})
		s.logInfo("cancelling goal…")
		return s, nil
	}
	return s, nil
}

func (s *session) handleListKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
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
	case modalMemory:
		items = s.memoryItems()
	case modalReasoning:
		items = s.reasoningItems()
	case modalProviders:
		items = s.providerItems()
	case modalLogout:
		items = s.logoutItems()
	case modalSettings:
		items = s.settingsHubItems()
	case modalApproval:
		items = s.approvalItems()
	case modalSandbox:
		items = s.sandboxItems()
	case modalAutoCompact:
		items = s.autoCompactItems()
	case modalNoNetwork:
		items = s.noNetworkItems()
	case modalMouseWheel:
		items = s.mouseWheelItems()
	}
	idx := filterList(items, s.modal.filter)
	n := len(idx)

	switch {
	case msg.String() == "up" || s.kbAny(msg, "nav_up", "nav_up_alt"):
		// hardcoded "up" fallback + keymap binding — stays usable even if nav is disabled
		if n > 0 {
			s.modal.cursor = (s.modal.cursor - 1 + n) % n
		}
	case msg.String() == "down" || s.kbAny(msg, "nav_down", "nav_down_alt"):
		// hardcoded "down" fallback + keymap binding
		if n > 0 {
			s.modal.cursor = (s.modal.cursor + 1) % n
		}
	case msg.String() == "enter" || s.kb(msg, "select"):
		// hardcoded "enter" fallback + keymap binding — never trap yourself out of selecting
		if n == 0 {
			return s, nil
		}
		if s.modal.cursor >= n {
			s.modal.cursor = 0
		}
		abs := idx[s.modal.cursor]
		return s.executeListSelect(abs)
	case msg.String() == "backspace":
		if len(s.modal.filter) > 0 {
			r := []rune(s.modal.filter)
			s.modal.filter = string(r[:len(r)-1])
			s.modal.cursor = 0
		}
	case s.kb(msg, "filter_clear"):
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
	case modalSettings:
		// Hub entry → open the dedicated modal / run the command for that option.
		items := s.settingsHubItems()
		if abs < 0 || abs >= len(items) {
			s.closeModal()
			return s, nil
		}
		label := items[abs].label
		s.closeModal()
		return s, s.dispatchSettingsCommand(label)
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
		// Toggle mode: enter flips enable/disable (modal stays open).
		// Remove mode (/plugin-remove): enter uninstalls and stays open.
		if s.pluginPickerMode == pluginModeRemove {
			s.removePlugin(abs)
			if s.modal.cursor >= len(sPluginStore) && s.modal.cursor > 0 {
				s.modal.cursor--
			}
		} else {
			s.togglePlugin(abs)
		}
		return s, nil
	case modalMemory:
		// Enter forgets the selected memory and drops it from the local list.
		items := s.memoryItems()
		if abs >= 0 && abs < len(items) {
			id := items[abs].meta
			if id != "" && id != "?" {
				s.sendCore(map[string]any{"type": "forget_memory", "id": id})
				s.logInfo("forgetting memory " + id)
				// Drop from cache so the row vanishes without a re-fetch.
				if abs < len(s.memoryList) {
					s.memoryList = append(s.memoryList[:abs], s.memoryList[abs+1:]...)
				}
				if s.modal.cursor >= len(s.memoryList) && s.modal.cursor > 0 {
					s.modal.cursor--
				}
			}
		}
		if len(s.memoryList) == 0 {
			s.closeModal()
		}
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
	case modalApproval:
		items := s.approvalItems()
		if abs >= 0 && abs < len(items) {
			mode := items[abs].label
			s.applyApprovalMode(mode)
		}
		s.closeModal()
		return s, nil
	case modalSandbox:
		items := s.sandboxItems()
		if abs >= 0 && abs < len(items) {
			mode := items[abs].label
			s.settings.Sandbox = mode
			_ = s.settings.save()
			s.logInfo(fmt.Sprintf("sandbox: %s (applies on next launch)", mode))
		}
		s.closeModal()
		return s, nil
	case modalAutoCompact:
		items := s.autoCompactItems()
		if abs >= 0 && abs < len(items) {
			on := items[abs].label == "on"
			s.coreAutoCompact = on
			s.sendCore(map[string]any{"type": "set_config", "key": "auto_compact", "value": on})
			s.logInfo(fmt.Sprintf("auto-compact: %s", boolStr(on)))
		}
		s.closeModal()
		return s, nil
	case modalNoNetwork:
		items := s.noNetworkItems()
		if abs >= 0 && abs < len(items) {
			on := items[abs].label == "on"
			s.settings.NoNetwork = on
			_ = s.settings.save()
			s.logInfo(fmt.Sprintf("no-network: %s (applies on next launch)", boolStr(on)))
		}
		s.closeModal()
		return s, nil
	case modalMouseWheel:
		items := s.mouseWheelItems()
		if abs >= 0 && abs < len(items) {
			on := items[abs].label == "on"
			s.settings.MouseWheel = on
			_ = s.settings.save()
			if on {
				s.logInfo("mouse wheel: on (hold Shift to select/copy text)")
			} else {
				s.logInfo("mouse wheel: off (click-drag to select/copy text)")
			}
			s.invalidateAll()
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

// applyApprovalMode persists and sends the approval gate mode to the core.
func (s *session) applyApprovalMode(mode string) {
	s.sendCore(map[string]any{"type": "set_approval", "mode": mode})
	s.settings.Approval = mode
	_ = s.settings.save()
	s.logInfo("approval: " + mode)
}

// dispatchSettingsCommand opens the dedicated modal (or runs the command) for
// a settings-hub entry. Shared by the hub list and runCommandByIndex.
func (s *session) dispatchSettingsCommand(label string) tea.Cmd {
	switch label {
	case "/login":
		s.openLoginPicker()
	case "/key":
		s.openAPIKeyModal()
	case "/approval", "/approvals":
		s.openApprovalPicker()
	case "/reasoning":
		s.openReasoningPicker()
	case "/theme":
		s.openThemePicker()
	case "/bash-timeout":
		s.openBashTimeoutModal()
	case "/auto-compact":
		s.openAutoCompactPicker()
	case "/sandbox":
		s.openSandboxPicker()
	case "/no-network":
		s.openNoNetworkPicker()
	case "/mouse-wheel":
		s.openMouseWheelPicker()
	case "/idle-timeout":
		s.openIdleTimeoutModal()
	case "/max-session-tokens":
		s.openMaxSessionTokensModal()
	case "/keybinds":
		s.openKeybindsModal()
	default:
		return s.handleUserLine(label)
	}
	return nil
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
		// OAuth-only presets (empty EnvVar, e.g. xAI SuperGrok): never prompt
		// for an API key. Logged-in → switch active provider; otherwise start
		// the device-code / browser OAuth flow.
		oauthOnly := preset.SupportsOauth && preset.EnvVar == ""
		if oauthOnly {
			if preset.LoggedIn {
				s.settings.ActiveProvider = name
				_ = s.settings.save()
				s.sendCore(map[string]any{"type": "set_provider", "name": name})
				s.logInfo("switching provider: " + preset.Label)
				s.closeModal()
				return s, nil
			}
			s.sendCore(map[string]any{"type": "login_oauth", "preset": name})
			s.logInfo("OAuth login: " + preset.Label + " — follow the prompt to log in")
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
		// No key: if the preset supports an OAuth subscription login (Gemini/
		// Claude), run it in-browser — no official CLI or API key needed. Otherwise
		// (Umans/Codex) prompt for an API key to paste.
		if preset.SupportsOauth {
			s.sendCore(map[string]any{"type": "login_oauth", "preset": name})
			s.logInfo("OAuth login: " + preset.Label + " — follow the prompt to log in")
			s.closeModal()
			return s, nil
		}
		// No key anywhere and no OAuth flow: prompt the user to paste one inline.
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
	// Optimistic UI: clear logged-in until core's provider_changed/presets arrive.
	// Without this the picker can still show ✓ while the OAuth token is gone.
	if p := s.presetByID(name); p != nil {
		p.LoggedIn = false
		p.HasKey = false
		p.Configured = false
	}
	if s.activeProvider == name {
		s.authed = false
		s.providerHasKey = false
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
		desc:  "paste your key, then Enter (Esc to cancel)",
	}}, true)
}
func (s *session) openOauthCodeModal() {
	s.modal.kind = modalOauthCode
	s.modal.editing = true
	ti := textinput.New()
	ti.Prompt = ""
	ti.Placeholder = "paste code or full localhost:51121 redirect URL"
	ti.Focus()
	s.modal.editBuf = ti
}

// renderOauthCodeModal renders the "paste your Google OAuth code" box. The long
// auth code is awkward to paste inline after /oauth-code (the command input
// mangles/truncates it), so bare /oauth-code opens this modal — a focused text
// field the user pastes into, then Enter to submit.
func (s *session) renderOauthCodeModal() string {
	val := s.modal.editBuf.Value()
	return s.renderListModal("Paste Google OAuth Code", []listItem{{
		label: val,
		desc:  "paste the code (or full localhost:51121 URL) from the browser redirect, then Enter (Esc to cancel)",
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
	case "/key":
		s.openAPIKeyModal()
		return nil
	case "/model":
		s.openModelPicker()
		return nil
	case "/approval", "/approvals":
		s.openApprovalPicker()
		return nil
	case "/reasoning":
		s.openReasoningPicker()
		return nil
	case "/bash-timeout":
		s.openBashTimeoutModal()
		return nil
	case "/auto-compact":
		s.openAutoCompactPicker()
		return nil
	case "/sandbox":
		s.openSandboxPicker()
		return nil
	case "/no-network":
		s.openNoNetworkPicker()
		return nil
	case "/mouse-wheel":
		s.openMouseWheelPicker()
		return nil
	case "/idle-timeout":
		s.openIdleTimeoutModal()
		return nil
	case "/max-session-tokens":
		s.openMaxSessionTokensModal()
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
	case "/keybinds":
		s.openKeybindsModal()
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
// Value-edit modals + helpers (API key, timeouts, etc.)
// ---------------------------------------------------------------------------

func boolStr(b bool) string {
	if b {
		return "on"
	}
	return "off"
}

// humanTokens renders a token count compactly (e.g. 1.2k, 47k, 128k) for the
// /context breakdown modal.
func humanTokens(n uint64) string {
	if n < 1000 {
		return fmt.Sprintf("%d", n)
	}
	if n < 1_000_000 {
		return fmt.Sprintf("%.1fk", float64(n)/1000)
	}
	return fmt.Sprintf("%.1fM", float64(n)/1_000_000)
}

func (s *session) handleSettingsEditKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch {
	case s.kb(msg, "close"):
		// Standalone edit modals (oauth code, value edit) dismiss entirely.
		// Login key capture returns to the provider list so the user can pick
		// another preset.
		if s.modal.kind == modalOauthCode || s.modal.kind == modalValueEdit {
			s.closeModal()
			return s, nil
		}
		s.pendingLogin = ""
		s.modal.editing = false
		return s, nil
	case msg.String() == "enter" || s.kb(msg, "select"):
		// hardcoded "enter" fallback so committing a pasted key can't be trapped
		// by an unbound select binding (mirrors list modals).
		return s.commitEditField()
	}
	var cmd tea.Cmd
	s.modal.editBuf, cmd = s.modal.editBuf.Update(msg)
	return s, cmd
}

func (s *session) commitEditField() (tea.Model, tea.Cmd) {
	// /oauth-code modal: the user pasted the authorization code into the edit
	// buffer (the long Google code is awkward to paste inline after the
	// command). Send it to the core — which holds the stashed PKCE verifier and
	// does the exchange — then close the modal.
	if s.modal.kind == modalOauthCode {
		code := strings.TrimSpace(s.modal.editBuf.Value())
		s.modal.editing = false
		s.closeModal()
		if code == "" {
			return s, nil
		}
		s.sendCore(map[string]any{"type": "oauth_code", "code": code})
		s.logInfo("submitting OAuth code…")
		return s, nil
	}
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
	// Dedicated value-edit modals (/key, /bash-timeout, /idle-timeout, …).
	if s.modal.kind == modalValueEdit {
		return s.commitValueEdit()
	}
	s.modal.editing = false
	return s, nil
}

// commitValueEdit applies a free-form setting from modalValueEdit and closes.
func (s *session) commitValueEdit() (tea.Model, tea.Cmd) {
	val := strings.TrimSpace(s.modal.editBuf.Value())
	target := s.modal.editTarget
	s.modal.editing = false
	s.closeModal()
	switch target {
	case editTargetAPIKey:
		if val == "" {
			s.logError("no key entered")
			return s, nil
		}
		name := s.activeProvider
		if name == "" {
			name = "default"
		}
		if s.settings.ProviderKeys == nil {
			s.settings.ProviderKeys = map[string]string{}
		}
		s.settings.ProviderKeys[name] = val
		s.settings.APIKey = val
		_ = s.settings.save()
		s.sendCore(map[string]any{"type": "set_key", "provider": name, "api_key": val})
		s.logInfo(fmt.Sprintf("sending key for provider '%s'…", name))
	case editTargetBashTimeout:
		var n int
		if _, err := fmt.Sscanf(val, "%d", &n); err == nil && n > 0 {
			s.coreBashTimeout = n
			s.sendCore(map[string]any{"type": "set_config", "key": "bash_timeout_secs", "value": n})
			s.logInfo(fmt.Sprintf("bash timeout: %ds", n))
		} else {
			s.logError("bash timeout must be a positive integer (seconds)")
		}
	case editTargetIdleTimeout:
		var n int
		if _, err := fmt.Sscanf(val, "%d", &n); err == nil && n >= 10 {
			s.settings.IdleTimeout = n
			_ = s.settings.save()
			s.logInfo(fmt.Sprintf("idle timeout: %ds (applies on next launch)", n))
		} else {
			s.logError("idle timeout must be ≥ 10 seconds")
		}
	case editTargetMaxSessionTokens:
		var n int
		if _, err := fmt.Sscanf(val, "%d", &n); err == nil && n >= 0 {
			s.settings.MaxSessionTokens = n
			_ = s.settings.save()
			s.logInfo(fmt.Sprintf("max session tokens: %d (applies on next launch)", n))
		} else {
			s.logError("max session tokens must be ≥ 0 (0=unlimited)")
		}
	case editTargetRemember:
		if val == "" {
			s.logError("no memory text entered")
			return s, nil
		}
		s.sendCore(map[string]any{"type": "save_memory", "text": val})
		s.sendCore(map[string]any{"type": "refresh_memory"})
		s.logSuccess("memory saved")
	case editTargetAttach:
		if val == "" {
			s.logError("no image path entered")
			return s, nil
		}
		return s, s.sendAttach(val, "")
	case editTargetPluginInstall:
		if val == "" {
			s.logError("no plugin path entered")
			return s, nil
		}
		s.sendCore(map[string]any{"type": "install_plugin", "path": val})
		s.logInfo(fmt.Sprintf("installing plugin from %s…", val))
	case editTargetSteer:
		if val == "" {
			s.logError("no steer message entered")
			return s, nil
		}
		return s, s.sendSteer(val)
	case editTargetRun:
		return s, s.runSubagentRest(val, "single")
	case editTargetParallel:
		return s, s.runSubagentRest(val, "parallel")
	case editTargetChain:
		return s, s.runSubagentRest(val, "chain")
	case editTargetCompact:
		if val == "" {
			s.sendCore(map[string]any{"type": "compact"})
		} else {
			s.sendCore(map[string]any{"type": "compact", "instructions": val})
		}
		s.logInfo("forcing context compaction…")
	case "goal_revise":
		if val == "" {
			s.logError("revision feedback is empty")
			return s, nil
		}
		if len(s.models) == 0 {
			s.logError("no models loaded yet")
			return s, nil
		}
		s.sendCore(map[string]any{
			"type":             "revise_goal",
			"feedback":         val,
			"model":            s.models[s.modelIdx].ID,
			"reasoning_effort": s.settings.ReasoningEffort,
		})
		s.busy = true
		s.logInfo("revising goal plan…")
	}
	return s, nil
}

// providerFieldLabel renders the active provider's name + kind (e.g.
// "anthropic [anthropic]"). Shows "default" when none configured.
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

func (s *session) handleHelpKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
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

func (s *session) helpText() string {
	lines := s.helpKeybindLines()
	lines = append(lines,
		"",
		"Mouse & copy",
		"  click-drag selects/copies text (mouse off by default)",
		"  /mouse-wheel on  enables wheel scrolling",
		"  (hold Shift to select/copy while the mouse is on)",
		"",
		"Slash commands",
		"  (bare commands open modals; skills still take optional task text)",
		"  /login           log in / switch provider (OpenAI · Gemini · Claude · xAI · Qwen · OpenRouter · …)",
		"  /logout          log out of a provider",
		"  /oauth-code      paste OAuth code (SSH/headless Google login)",
		"  /key             set API key for active provider",
		"  /model           switch model",
		"  /approval        never | destructive | always",
		"  /reasoning       set reasoning effort (per model)",
		"  /theme           switch colour theme",
		"  /bash-timeout    bash tool timeout (seconds)",
		"  /auto-compact    auto context compaction on/off",
		"  /sandbox         sandbox mode (none · firejail)",
		"  /no-network      block network in sandbox on/off",
		"  /mouse-wheel     mouse-wheel scrolling on/off",
		"  /idle-timeout    idle timeout (seconds)",
		"  /max-session-tokens  max session tokens (0=unlimited)",
		"  /reset            wipe conversation + session file",
		"  /clear            clear view (keep session file)",
		"  /undo             drop last turn",
		"  /compact          force context compaction",
		"  /sessions         open session picker",
		"  /new              start a fresh session file",
		"  /stats            token + turn totals",
		"  /context          token-usage breakdown (top consumers)",
		"  /usage            provider plan limits (5h · weekly · …)",
		"  /abort            stop running turn",
		"  /steer            steer an in-flight turn",
		"  /settings         settings hub (opens dedicated modals)",
		"  /keybinds         view & customize keybindings",
		"  /copy             copy last assistant reply",
		"  /attach           send an image (vision)",
		"  /vision           configure vision models & handoff target",
		"  /remember         save a memory note",
		"  /memory · /forget list / forget memories",
		"  /plugin-install   install a plugin from a directory",
		"  /plugin-config    enable / disable plugins",
		"  /plugin-remove    uninstall a plugin",
		"  /goal             goal mode — plan & deploy subagents (modal)",
		"  /cancel-goal      cancel active goal mode",
		"  /run · /parallel · /chain  subagent delegation",
		"  /skill:<name> [task]  apply a skill (task optional)",
		"",
		"Settings persist to ~/.config/catalyst-code/settings.json",
		"Config (core) persists to ~/.config/catalyst-code/config.json",
		"",
		"Custom providers (OpenAI- & Anthropic-compatible endpoints)",
		"  Define named providers in the core config file's `providers` array:",
		"    { \"name\": \"anthropic\", \"kind\": \"anthropic\",",
		"      \"base_url\": \"https://api.anthropic.com/v1\",",
		"      \"api_key_env\": \"ANTHROPIC_API_KEY\" }",
		"    { \"name\": \"local\", \"kind\": \"openai\", \"base_url\": \"http://localhost:11434/v1\" }",
		"  Select one at startup with `--provider <name>` or UMANS_ACTIVE_PROVIDER.",
		"  Switch at runtime: /login (or /settings → /login).",
		"  Each provider keeps its own key (/key stores per-provider).",
	)
	return strings.Join(lines, "\n")
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
		title := "Plugins"
		if s.pluginPickerMode == pluginModeRemove {
			title = "Uninstall Plugin"
		}
		return s.renderListModal(title, s.pluginItems(), false)
	case modalMemory:
		return s.renderListModal("Memories (enter to forget)", s.memoryItems(), true)
	case modalReasoning:
		return s.renderListModal("Reasoning Effort", s.reasoningItems(), true)
	case modalProviders:
		if s.modal.editing {
			return s.renderLoginKeyBox()
		}
		return s.renderListModal("Log in / switch provider", s.providerItems(), true)
	case modalOauthCode:
		return s.renderOauthCodeModal()
	case modalLogout:
		return s.renderListModal("Log out", s.logoutItems(), true)
	case modalVision:
		return s.renderListModal("Vision Models", s.visionItems(), true)
	case modalSettings:
		return s.renderListModal("Settings", s.settingsHubItems(), true)
	case modalApproval:
		return s.renderListModal("Approval Mode", s.approvalItems(), false)
	case modalSandbox:
		return s.renderListModal("Sandbox", s.sandboxItems(), false)
	case modalAutoCompact:
		return s.renderListModal("Auto Compact", s.autoCompactItems(), false)
	case modalNoNetwork:
		return s.renderListModal("No Network", s.noNetworkItems(), false)
	case modalMouseWheel:
		return s.renderListModal("Mouse Wheel", s.mouseWheelItems(), false)
	case modalValueEdit:
		return s.renderValueEditModal()
	case modalGoal:
		return s.renderGoalModal()
	case modalGoalPlan:
		return s.renderGoalPlanModal()
	case modalHelp:
		return s.renderHelpModal()
	case modalKeybinds:
		return s.renderKeybindsModal()
	case modalContext:
		return s.renderContextModal()
	case modalUsage:
		return s.renderUsageModal()
	}
	return ""
}

func (s *session) renderGoalModal() string {
	w := s.modalWidth(78)
	d := s.goalDraft
	row := func(idx int, label, value string) string {
		marker := "  "
		style := dimStyle
		if d.field == idx {
			marker = "▸ "
			style = accentStyle
		}
		return style.Render(marker+label) + " " + baseStyle.Render(value)
	}
	var lines []string
	lines = append(lines, accentStyle.Render("◆ Goal Mode"))
	lines = append(lines, separatorStyle.Render(strings.Repeat("─", w-2)))

	goalVal := d.goal
	if d.editing && d.field == goalFieldGoal {
		goalVal = s.modal.editBuf.Value()
		if goalVal == "" {
			goalVal = s.modal.editBuf.Placeholder
		}
	}
	if goalVal == "" {
		goalVal = "(empty — enter to edit)"
	}
	// Truncate long goals for the form view.
	if len([]rune(goalVal)) > w-14 {
		goalVal = string([]rune(goalVal)[:w-15]) + "…"
	}
	lines = append(lines, row(goalFieldGoal, "Goal", goalVal))
	lines = append(lines, row(goalFieldConcurrency, "Concurrency", fmt.Sprintf("%d  (←/→)", d.concurrency)))
	lines = append(lines, row(goalFieldMaxTasks, "Max tasks", fmt.Sprintf("%d  (←/→)", d.maxTasks)))

	// Providers multi-select
	provs := s.goalProviderOptions()
	provLabel := "all"
	if len(d.allowedProviders) > 0 {
		var sel []string
		for _, p := range provs {
			if d.allowedProviders[p] {
				sel = append(sel, p)
			}
		}
		if len(sel) == 0 {
			provLabel = "all"
		} else {
			provLabel = strings.Join(sel, ", ")
		}
	}
	lines = append(lines, row(goalFieldProviders, "Providers", provLabel+"  (space toggle)"))
	if d.field == goalFieldProviders {
		if len(provs) == 0 {
			lines = append(lines, dimStyle.Render("    (no providers logged in)"))
		}
		for i, p := range provs {
			mark := " "
			if d.allowedProviders[p] {
				mark = "✓"
			}
			cur := "  "
			if i == d.listCursor {
				cur = "▸ "
			}
			lines = append(lines, dimStyle.Render(fmt.Sprintf("  %s[%s] %s", cur, mark, p)))
		}
	}

	// Models multi-select
	mods := s.goalModelOptions()
	modLabel := "all"
	if len(d.allowedModels) > 0 {
		var sel []string
		for _, m := range mods {
			if d.allowedModels[m] {
				sel = append(sel, m)
			}
		}
		if len(sel) > 0 {
			modLabel = strings.Join(sel, ", ")
			if len(modLabel) > w-20 {
				modLabel = fmt.Sprintf("%d selected", len(sel))
			}
		}
	}
	lines = append(lines, row(goalFieldModels, "Models", modLabel+"  (space toggle)"))
	if d.field == goalFieldModels {
		// Show a window of models around the cursor.
		start := d.listCursor - 3
		if start < 0 {
			start = 0
		}
		end := start + 7
		if end > len(mods) {
			end = len(mods)
		}
		if len(mods) == 0 {
			lines = append(lines, dimStyle.Render("    (no models)"))
		}
		for i := start; i < end; i++ {
			m := mods[i]
			mark := " "
			if d.allowedModels[m] {
				mark = "✓"
			}
			cur := "  "
			if i == d.listCursor {
				cur = "▸ "
			}
			lines = append(lines, dimStyle.Render(fmt.Sprintf("  %s[%s] %s", cur, mark, m)))
		}
	}

	review := "off — deploy after plan"
	if d.reviewBeforeDeploy {
		review = "on — wait for approval"
	}
	lines = append(lines, row(goalFieldReview, "Review plan", review+"  (space)"))

	advLabel := "off"
	if d.advanced {
		advLabel = "on"
	}
	lines = append(lines, row(goalFieldAdvanced, "Advanced", advLabel+"  (space) — role models & per-model concurrency"))

	if d.advanced {
		roleVal := func(m string) string {
			if m == "" {
				return "(default)"
			}
			return m
		}
		lines = append(lines, row(goalFieldPlanner, "  Planner model", roleVal(d.plannerModel)+"  (←/→)"))
		lines = append(lines, row(goalFieldWorker, "  Worker model", roleVal(d.workerModel)+"  (←/→)"))
		lines = append(lines, row(goalFieldReviewer, "  Reviewer model", roleVal(d.reviewerModel)+"  (←/→)"))

		// Per-model concurrency list
		concOpts := s.goalModelConcOptions()
		lines = append(lines, row(goalFieldModelConc, "  Model concurrency", "←/→ adjust · empty = global"))
		if d.field == goalFieldModelConc {
			if len(concOpts) == 0 {
				lines = append(lines, dimStyle.Render("    (no models)"))
			}
			start := d.listCursor - 3
			if start < 0 {
				start = 0
			}
			end := start + 7
			if end > len(concOpts) {
				end = len(concOpts)
			}
			for i := start; i < end; i++ {
				id := concOpts[i]
				cap := d.concurrency
				if v, ok := d.modelConcurrency[id]; ok && v > 0 {
					cap = v
				}
				cur := "  "
				if i == d.listCursor {
					cur = "▸ "
				}
				lines = append(lines, dimStyle.Render(fmt.Sprintf("  %s%s  %d/%d", cur, id, cap, d.concurrency)))
			}
		}
	}

	startLabel := "Start goal"
	if d.field == goalFieldStart {
		lines = append(lines, accentStyle.Render("▸ ▶ "+startLabel+"  (enter)"))
	} else {
		lines = append(lines, dimStyle.Render("  ▶ "+startLabel))
	}

	lines = append(lines, "")
	lines = append(lines, dimStyle.Render("  ↑↓ fields · space toggle · ←/→ cycle · ctrl+enter submit · esc cancel"))
	return modalBox(w, strings.Join(lines, "\n"))
}

func (s *session) renderGoalPlanModal() string {
	w := s.modalWidth(78)
	var lines []string
	lines = append(lines, accentStyle.Render("◆ Goal Plan Ready"))
	lines = append(lines, separatorStyle.Render(strings.Repeat("─", w-2)))
	if s.goalState != nil {
		g := s.goalState.Goal
		if len([]rune(g)) > w-10 {
			g = string([]rune(g)[:w-11]) + "…"
		}
		lines = append(lines, baseStyle.Render("  "+g))
		lines = append(lines, "")
		for i, p := range s.goalState.Prompts {
			title := p.Title
			if title == "" {
				title = p.StepID
			}
			lines = append(lines, fmt.Sprintf("  %d. [%s] %s", i+1, p.Agent, title))
		}
		if len(s.goalState.Prompts) == 0 {
			lines = append(lines, dimStyle.Render("  (no steps in plan)"))
		}
	} else {
		lines = append(lines, dimStyle.Render("  (no goal_state yet)"))
	}
	lines = append(lines, "")
	lines = append(lines, dimStyle.Render("  a/enter approve · r revise · q/esc cancel"))
	return modalBox(w, strings.Join(lines, "\n"))
}

// fitListRow builds a single-line list row — marker + label + desc — that
// fits width visible columns. When both fit, the description is kept whole
// and the label is truncated (it is the least essential for session-style
// rows like "12 msgs · 2h ago"). When the description is too long to fit
// beside any label, the label is kept (truncated) and the description is
// truncated to the remaining space — the command name must stay visible so
// name + description always share one line. The marker is already styled;
// markerW is its visible width.
func fitListRow(marker, label, desc string, markerW, width int) string {
	budget := width - markerW
	if budget < 0 {
		budget = 0
	}
	if d := len([]rune(desc)); d > 0 {
		if 2+d <= budget {
			label = truncateFit(label, budget-2-d)
		} else {
			// desc is too long to fit whole beside any label: keep a truncated
			// label (the command name is what the user selects, so it must stay
			// visible) and truncate the desc to the remaining space so name +
			// description stay on a single line instead of the label vanishing.
			maxLabel := budget / 3 // cap so a long label can't starve the desc
			if maxLabel < 1 {
				maxLabel = 1
			}
			label = truncateFit(label, maxLabel)
			desc = truncateFit(desc, budget-2-len([]rune(label)))
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
		if s.pluginPickerMode == pluginModeRemove {
			footer = "  ↑↓ navigate · enter uninstall · esc close"
		} else {
			footer = "  ↑↓ navigate · enter toggle enable/disable · esc close"
		}
	}
	if s.modal.kind == modalMemory {
		footer = "  ↑↓ navigate · enter forget · esc close"
	}
	if s.modal.kind == modalVision {
		footer = "  ↑↓ navigate · space toggle vision · enter set target · esc close"
	}
	lines = append(lines, truncStyle.Render(dimStyle.Render(footer)))
	body := strings.Join(lines, "\n")
	return modalBox(w, body)
}

// renderUsageModal renders the /usage provider plan/rate-limit report: which
// provider the selected model routes to, plan name, and one row per window
// (5-hour, weekly, concurrency, …). Read-only display.
func (s *session) renderUsageModal() string {
	w := s.modalWidth(78)
	var lines []string
	lines = append(lines, accentStyle.Render("◆ Provider Usage"))
	lines = append(lines, separatorStyle.Render(strings.Repeat("─", w-2)))
	ur := s.usageReport
	if ur == nil {
		lines = append(lines, mutedStyle.Render("  loading…"))
		lines = append(lines, "")
		lines = append(lines, dimStyle.Render("  esc close"))
		return modalBox(w, strings.Join(lines, "\n"))
	}
	// Header: provider + model.
	prov := ur.Provider
	if prov == "" {
		prov = "unknown"
	}
	header := fmt.Sprintf("%s", accentStyle.Render(prov))
	if ur.Plan != "" {
		header += mutedStyle.Render(" · ") + baseStyle.Render(ur.Plan)
	}
	lines = append(lines, "  "+header)
	if ur.Model != "" {
		lines = append(lines, "  "+dimStyle.Render("model ")+ur.Model)
	}
	lines = append(lines, "")

	if !ur.Available {
		msg := ur.Message
		if msg == "" {
			msg = "Usage stats are not available for this provider."
		}
		// Word-wrap the message to the modal width.
		for _, ln := range wrapUsageText(msg, w-4) {
			lines = append(lines, "  "+mutedStyle.Render(ln))
		}
		lines = append(lines, "")
		lines = append(lines, dimStyle.Render("  esc close"))
		return modalBox(w, strings.Join(lines, "\n"))
	}

	if len(ur.Windows) == 0 {
		lines = append(lines, mutedStyle.Render("  no limit windows reported"))
	} else {
		barWidth := w - 28
		if barWidth < 12 {
			barWidth = 12
		}
		if barWidth > 40 {
			barWidth = 40
		}
		for _, win := range ur.Windows {
			label := win.Label
			if label == "" {
				label = win.ID
			}
			lines = append(lines, "  "+baseStyle.Render(label))
			usedStr, barRatio := formatUsageAmount(win)
			// Progress bar only when a limit is available (ratio > 0 or limit set).
			// Unlimited rows skip the empty bar so they don't look like 0% used.
			row := "    "
			if win.Limit != nil && *win.Limit > 0 {
				row += renderUsageBar(barRatio, barWidth)
				if usedStr != "" {
					row += "  "
				}
			}
			if usedStr != "" {
				row += accentStyle.Render(usedStr)
			}
			lines = append(lines, row)
			if win.Detail != "" {
				lines = append(lines, "    "+dimStyle.Render(win.Detail))
			}
		}
	}
	if ur.Message != "" {
		lines = append(lines, "")
		for _, ln := range wrapUsageText(ur.Message, w-4) {
			lines = append(lines, "  "+mutedStyle.Render(ln))
		}
	}
	lines = append(lines, "")
	lines = append(lines, dimStyle.Render("  esc close"))
	return modalBox(w, strings.Join(lines, "\n"))
}

// formatUsageAmount returns a human used/limit string and a 0–1 bar ratio.
// Percentage is only shown when a positive limit is available; unlimited /
// unknown ceilings never get a "%" suffix (just the raw used amount).
func formatUsageAmount(win usageWindow) (string, float64) {
	unit := strings.ToLower(win.Unit)
	used := 0.0
	hasUsed := win.Used != nil
	if hasUsed {
		used = *win.Used
	}
	limit := 0.0
	hasLimit := win.Limit != nil && *win.Limit > 0
	if hasLimit {
		limit = *win.Limit
	}

	switch unit {
	case "percent":
		// unit==percent already encodes utilization 0–100 (limit is 100).
		// Only show "%" when we have a used value (the percentage itself).
		if !hasUsed {
			return "", 0
		}
		ratio := used / 100.0
		if ratio < 0 {
			ratio = 0
		}
		if ratio > 1 {
			ratio = 1
		}
		return fmt.Sprintf("%.0f%% used", used), ratio
	default:
		// count-like units: sessions, requests, tokens, credits, count
		if hasUsed && hasLimit {
			ratio := used / limit
			if ratio < 0 {
				ratio = 0
			}
			if ratio > 1 {
				ratio = 1
			}
			pct := ratio * 100
			// "885 / 15.0k (6%)" — percentage only when limit is known.
			return fmt.Sprintf("%s / %s (%.0f%%)",
				formatUsageNumber(used), formatUsageNumber(limit), pct), ratio
		}
		if hasUsed {
			// No limit → never show a percentage; just the used amount.
			return formatUsageNumber(used) + " used", 0
		}
		if hasLimit {
			return formatUsageNumber(limit) + " limit", 0
		}
		return "", 0
	}
}

func formatUsageNumber(n float64) string {
	if n >= 1_000_000 {
		return fmt.Sprintf("%.1fM", n/1_000_000)
	}
	if n >= 1_000 {
		return fmt.Sprintf("%.1fk", n/1_000)
	}
	// Prefer integers when close.
	if n == float64(int64(n)) {
		return fmt.Sprintf("%d", int64(n))
	}
	return fmt.Sprintf("%.1f", n)
}

// renderUsageBar draws a filled/empty progress bar using block characters.
func renderUsageBar(ratio float64, width int) string {
	if width < 4 {
		width = 4
	}
	if ratio < 0 {
		ratio = 0
	}
	if ratio > 1 {
		ratio = 1
	}
	filled := int(ratio*float64(width) + 0.5)
	if filled > width {
		filled = width
	}
	bar := strings.Repeat("█", filled) + strings.Repeat("░", width-filled)
	// Color by pressure: green < 70%, yellow < 90%, red otherwise.
	style := successStyle
	if ratio >= 0.9 {
		style = errStyle
	} else if ratio >= 0.7 {
		style = warnStyle
	}
	return style.Render(bar)
}

func wrapUsageText(s string, width int) []string {
	if width < 20 {
		width = 20
	}
	words := strings.Fields(s)
	if len(words) == 0 {
		return nil
	}
	var lines []string
	cur := words[0]
	for _, w := range words[1:] {
		if len(cur)+1+len(w) <= width {
			cur += " " + w
		} else {
			lines = append(lines, cur)
			cur = w
		}
	}
	lines = append(lines, cur)
	return lines
}

// renderContextModal renders the /context token-usage breakdown: total/window,
// per-role buckets, and the top token consumers. Read-only display.
func (s *session) renderContextModal() string {
	w := s.modalWidth(78)
	var lines []string
	lines = append(lines, accentStyle.Render("◆ Context Breakdown"))
	lines = append(lines, separatorStyle.Render(strings.Repeat("─", w-2)))
	cb := s.ctxBreakdown
	if cb == nil {
		lines = append(lines, mutedStyle.Render("  no data"))
		lines = append(lines, "")
		lines = append(lines, dimStyle.Render("  esc close"))
		return modalBox(w, strings.Join(lines, "\n"))
	}
	lines = append(lines, fmt.Sprintf("%s: %s / %s  (%s%%)",
		baseStyle.Render("Total"),
		accentStyle.Render(humanTokens(cb.Total)),
		mutedStyle.Render(humanTokens(cb.Window)),
		accentStyle.Render(fmt.Sprintf("%d", cb.Pct))))
	lines = append(lines, fmt.Sprintf("%s: %d", baseStyle.Render("Messages"), cb.Messages))
	// Per-role buckets.
	if len(cb.ByRole) > 0 {
		lines = append(lines, "")
		lines = append(lines, dimStyle.Render("  by role:"))
		// Stable order: system, user, assistant, tool, then any others.
		order := []string{"system", "user", "assistant", "tool"}
		seen := map[string]bool{}
		for _, r := range order {
			if v, ok := cb.ByRole[r]; ok {
				lines = append(lines, fmt.Sprintf("    %-9s %s", r, humanTokens(v)))
				seen[r] = true
			}
		}
		for r, v := range cb.ByRole {
			if !seen[r] {
				lines = append(lines, fmt.Sprintf("    %-9s %s", r, humanTokens(v)))
			}
		}
	}
	// Top consumers.
	if len(cb.TopConsumers) > 0 {
		lines = append(lines, "")
		lines = append(lines, dimStyle.Render("  top consumers:"))
		for _, c := range cb.TopConsumers {
			prev := c.Preview
			maxRunes := w - 34
			if maxRunes < 20 {
				maxRunes = 20
			}
			if len([]rune(prev)) > maxRunes {
				prev = string([]rune(prev)[:maxRunes]) + "…"
			}
			lines = append(lines, fmt.Sprintf("    #%d %-9s %s  %s",
				c.Index, c.Role, humanTokens(c.Tokens), mutedStyle.Render(prev)))
		}
	}
	lines = append(lines, "")
	lines = append(lines, dimStyle.Render("  esc close"))
	return modalBox(w, strings.Join(lines, "\n"))
}

// renderValueEditModal renders a free-form edit box for a single setting
// (API key, bash timeout, idle timeout, max session tokens). Built by hand
// (not via renderListModal) so the title is never treated as a list filter.
func (s *session) renderValueEditModal() string {
	w := s.modalWidth(72)
	title := s.modal.filter // openValueEditModal stores the title here
	if title == "" {
		title = "Edit value"
	}
	val := s.modal.editBuf.Value()
	// Mask API keys while typing.
	display := val
	if s.modal.editTarget == editTargetAPIKey && val != "" {
		display = strings.Repeat("•", len(val))
	}
	if display == "" && s.modal.editBuf.Placeholder != "" {
		display = s.modal.editBuf.Placeholder
	}
	var lines []string
	lines = append(lines, accentStyle.Render("◆ "+title))
	lines = append(lines, separatorStyle.Render(strings.Repeat("─", w-2)))
	lines = append(lines, accentStyle.Render("▸ ")+baseStyle.Render(display))
	lines = append(lines, "")
	lines = append(lines, dimStyle.Render("  type a value · enter save · esc cancel"))
	return modalBox(w, strings.Join(lines, "\n"))
}

func (s *session) renderHelpModal() string {
	w := s.modalWidth(80)
	h := s.height - 6
	if h < 6 {
		h = 6
	}
	allLines := strings.Split(s.helpText(), "\n")
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

func isPrintable(msg tea.KeyPressMsg) bool {
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
