package main

import (
	"fmt"
	"strings"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

// ---------------------------------------------------------------------------
// Customizable keybindings
//
// Every key the TUI matches is routed through a named action in a keymap so the
// user can rebind it via /keybinds. The keymap is a map[actionName]canonicalKey
// where canonicalKey is the string form that tea.KeyPressMsg.String() produces
// (e.g. "ctrl+t", "enter", "pgup"). Defaults are declared in keybindDefs; user
// overrides are persisted in settings.json and merged over the defaults on load.
//
// Comparison goes through s.kb(msg, action) which is case-insensitive for
// single-character keys (so "y" and "Y" both match an "approve" binding) and
// exact for multi-character key names.
// ---------------------------------------------------------------------------

// keybindDef is the master registry entry for one bindable action.
type keybindDef struct {
	Action  string // stable id, used as the keymap key and in settings.json
	Group   string // display group (Global, Scrolling, …)
	Desc    string // human-readable description
	Default string // default canonical key (tea.KeyPressMsg.String() form)
}

// keybindDefs is the complete, ordered list of every bindable action. This is
// the single source of truth — the /keybinds modal, the help text, and the
// default keymap all derive from it. Add a new action here and wire s.kb()
// at the call site; everything else updates automatically.
var keybindDefs = []keybindDef{
	// Global — work in every non-modal state.
	{"quit", "Global", "Quit the application", "ctrl+c"},
	{"toggle_reasoning", "Global", "Toggle reasoning-block collapse", "ctrl+t"},
	{"toggle_tool_output", "Global", "Expand / collapse last tool output", "ctrl+o"},
	{"command_palette", "Global", "Open command palette", "ctrl+p"},
	{"command_palette_alt", "Global", "Open command palette (alternate)", "ctrl+k"},
	{"reasoning_picker", "Global", "Set reasoning effort (per model)", "ctrl+r"},
	{"close", "Global", "Close modal / deny / abort / cancel", "esc"},

	// Scrolling — transcript viewport, every state.
	{"scroll_page_up", "Scrolling", "Scroll up one page", "pgup"},
	{"scroll_page_down", "Scrolling", "Scroll down one page", "pgdown"},
	{"scroll_line_up", "Scrolling", "Scroll up one line", "ctrl+up"},
	{"scroll_line_down", "Scrolling", "Scroll down one line", "ctrl+down"},
	{"scroll_top", "Scrolling", "Jump to top", "ctrl+home"},
	{"scroll_bottom", "Scrolling", "Jump to bottom", "ctrl+end"},

	// Navigation — modals, welcome screen, @-mention flyout.
	{"nav_up", "Navigation", "Move selection up", "up"},
	{"nav_down", "Navigation", "Move selection down", "down"},
	{"nav_up_alt", "Navigation", "Move selection up (alternate)", "k"},
	{"nav_down_alt", "Navigation", "Move selection down (alternate)", "j"},
	{"select", "Navigation", "Select / confirm / accept", "enter"},
	{"filter_clear", "Navigation", "Clear search filter", "ctrl+w"},

	// Input — chat box, history, steering.
	{"send", "Input", "Send message / queue follow-up / reply", "enter"},
	{"newline", "Input", "Insert a line break (multi-line input)", "shift+enter"},
	{"steer", "Input", "Steer (interrupt + redirect the model)", "ctrl+enter"},
	{"history_prev", "Input", "Recall previous input", "up"},
	{"history_next", "Input", "Recall next input", "down"},
	// paste_image reads an image from the local clipboard (wl-paste/xclip/
	// pngpaste/osascript/PowerShell). Over SSH the remote host usually has no
	// display, so prefer Ctrl/Cmd+V which delivers a bracketed PasteMsg — paths
	// and base64 image data are auto-detected. This keybind is for local GUI
	// sessions and reverse-forwarded clipboard helpers.
	{"paste_image", "Input", "Paste image from clipboard", "ctrl+shift+v"},
	{"detach_image", "Input", "Remove last attached image", "ctrl+shift+x"},

	// Approval — when a destructive action is pending.
	{"approve", "Approval", "Approve once", "y"},
	{"approve_always", "Approval", "Approve & stop asking", "a"},
	{"deny", "Approval", "Deny", "n"},

	// Mention — @-mention file flyout.
	{"mention_accept", "Mention", "Accept @-mention selection", "tab"},

	// Settings list navigation (kept for remaps / help; list modals also
	// accept bare up/down/enter/esc so they stay usable if these are unbound).
	{"field_next", "Settings", "Next list item (settings hub)", "tab"},
	{"field_prev", "Settings", "Previous list item (settings hub)", "shift+tab"},
	{"cycle_left", "Settings", "Cycle value left (legacy)", "left"},
	{"cycle_left_alt", "Settings", "Cycle value left alternate (legacy)", "h"},
	{"cycle_right", "Settings", "Cycle value right (legacy)", "right"},
	{"cycle_right_alt", "Settings", "Cycle value right alternate (legacy)", "l"},

	// Vision modal — toggle vision-capable models.
	{"vision_toggle", "Vision", "Toggle vision-capable", " "},
}

// keybindGroupOrder is the display order of groups in the /keybinds modal and
// help text. Derived from keybindDefs but fixed so reordering defs doesn't
// reshuffle the UI.
var keybindGroupOrder = []string{
	"Global", "Scrolling", "Navigation", "Input",
	"Approval", "Mention", "Settings", "Vision",
}

// defaultKeybinds builds the default action→key map from keybindDefs.
func defaultKeybinds() map[string]string {
	m := make(map[string]string, len(keybindDefs))
	for _, d := range keybindDefs {
		m[d.Action] = d.Default
	}
	return m
}

// effectiveKeybinds merges user overrides over the defaults. Unknown actions in
// the user map are dropped. An empty string is a VALID override meaning
// "disabled" — the action will never match any key (kb returns false for an
// empty binding), letting the user unbind alternate keys (e.g. clear ctrl+k so
// only ctrl+p opens the palette). This is the function called at startup to
// populate session.keybinds.
func effectiveKeybinds(user map[string]string) map[string]string {
	m := defaultKeybinds()
	for k, v := range user {
		if _, ok := m[k]; ok {
			m[k] = strings.TrimSpace(v) // "" = disabled
		}
	}
	return m
}

// caseInsensitiveActions are the actions whose single-character keys are
// matched case-insensitively (so "y" and "Y" both approve). This mirrors the
// original hardcoded behavior (case "y", "Y"). Navigation alts (j/k/h/l) are
// NOT case-folded so an uppercase letter typed into a search filter still
// goes to the filter instead of navigating.
var caseInsensitiveActions = map[string]bool{
	"approve":        true,
	"approve_always": true,
	"deny":           true,
}

// kb reports whether msg matches the key bound to action. Single-character keys
// in caseInsensitiveActions are compared case-insensitively (so "y" matches
// "Y"); everything else — including multi-character key names (ctrl+t, enter,
// pgup, …) and vim-style nav alts (j/k/h/l) — is compared exactly.
func (s *session) kb(msg tea.KeyPressMsg, action string) bool {
	key, ok := s.keybinds[action]
	if !ok || key == "" {
		return false
	}
	got := msg.String()
	// Bubble Tea v2 renders the space bar as "space" (v1 returned " "). The
	// vision_toggle default is stored as " ", so normalize both sides so existing
	// settings keep working and a freshly captured space binds consistently.
	got = spaceNorm(got)
	key = spaceNorm(key)
	if caseInsensitiveActions[action] && len([]rune(key)) == 1 {
		return strings.EqualFold(got, key)
	}
	return got == key
}

// spaceNorm maps the two space representations to one canonical form so kb()
// matches regardless of which Bubble Tea major version emitted the key string.
func spaceNorm(s string) string {
	if s == " " {
		return "space"
	}
	return s
}

// kbAny reports whether msg matches any of the named actions (shortcut for the
// common "primary OR alternate" pattern, e.g. command_palette / command_palette_alt).
func (s *session) kbAny(msg tea.KeyPressMsg, actions ...string) bool {
	for _, a := range actions {
		if s.kb(msg, a) {
			return true
		}
	}
	return false
}

// isBindableKey reports whether a canonical key string is acceptable as a
// binding. Rejects empty/blank strings and a few sentinel values that
// tea.KeyPressMsg can produce for non-physical-key events. Everything else (ctrl+*,
// enter, esc, pgup, single chars, …) is bindable.
func isBindableKey(key string) bool {
	if key == "" {
		return false
	}
	// bubbletea uses these sentinels for unknown/error keys.
	switch key {
	case "unknown", "error":
		return false
	}
	return true
}

// keybindLabel pretty-prints a canonical key for display (e.g. "ctrl+t" →
// "Ctrl+T", " " → "Space", "enter" → "Enter"). An empty key (disabled) renders
// as "—".
func keybindLabel(key string) string {
	if key == "" {
		return "—"
	}
	if key == " " {
		return "Space"
	}
	parts := strings.Split(key, "+")
	for i, p := range parts {
		switch p {
		case "ctrl", "alt", "shift":
			parts[i] = strings.ToUpper(p[:1]) + p[1:]
		default:
			if len(p) > 0 {
				parts[i] = strings.ToUpper(p[:1]) + p[1:]
			}
		}
	}
	return strings.Join(parts, "+")
}

// ---------------------------------------------------------------------------
// /keybinds modal
//
// A scrollable list of every bindable action with its current key. Enter starts
// capture mode (the next physical key becomes the binding); Backspace resets
// the selected action to its default; Delete clears (disables) a binding so
// it never matches — useful for removing alternate keys like ctrl+k. Esc
// closes (or cancels capture). Navigation/select/close also have hardcoded
// fallback keys (up/down/enter/esc) so the modal stays usable even if those
// bindings are disabled.
// ---------------------------------------------------------------------------

func (s *session) openKeybindsModal() {
	s.modal = newModal()
	s.modal.kind = modalKeybinds
	s.modal.cursor = 0
	s.modal.scroll = 0
}

// handleKeybindsKey drives the /keybinds modal. In capture mode (modal.editing)
// the next key is assigned to the selected action; otherwise the modal
// navigates and selects. Navigation/select/close have HARDCODED fallback keys
// (up/down/enter/esc) in addition to the keymap bindings, so the modal stays
// usable even if the user disables all nav/select/close bindings — you can
// never trap yourself inside the keybinds editor.
func (s *session) handleKeybindsKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	n := len(keybindDefs)

	if s.modal.editing {
		// Capture mode: assign the pressed key to the selected action.
		// Esc cancels capture without binding (so Esc itself can't be bound via
		// capture — it already defaults to `close`; use Backspace to reset).
		key := msg.String()
		if key == "esc" {
			s.modal.editing = false
			return s, nil
		}
		if !isBindableKey(key) {
			s.modal.editing = false
			s.logError("that key can't be bound")
			return s, nil
		}
		s.applyKeybind(s.modal.cursor, key)
		s.modal.editing = false
		return s, nil
	}

	// Esc is ALWAYS a guaranteed close (hardcoded) so the user can never trap
	// themselves: esc can't be bound to any other action (it cancels capture),
	// so it is always free to close the modal.
	if msg.String() == "esc" {
		s.closeModal()
		return s, nil
	}
	switch {
	case msg.String() == "up" || s.kbAny(msg, "nav_up", "nav_up_alt"):
		// hardcoded "up" fallback + keymap binding
		if n > 0 {
			s.modal.cursor = (s.modal.cursor - 1 + n) % n
		}
	case msg.String() == "down" || s.kbAny(msg, "nav_down", "nav_down_alt"):
		// hardcoded "down" fallback + keymap binding
		if n > 0 {
			s.modal.cursor = (s.modal.cursor + 1) % n
		}
	case msg.String() == "enter" || s.kb(msg, "select"):
		// hardcoded "enter" fallback + keymap binding — start key capture
		s.modal.editing = true
	case msg.String() == "backspace":
		s.resetKeybind(s.modal.cursor) // restore default key
	case msg.String() == "delete":
		s.clearKeybind(s.modal.cursor) // disable (unbind) the action
	case s.kbAny(msg, "close", "quit"):
		s.closeModal()
	}
	return s, nil
}

// keybindsInGroup returns the action names that share a group with the action
// at idx (excluding idx itself).
func keybindsInGroup(idx int) []string {
	if idx < 0 || idx >= len(keybindDefs) {
		return nil
	}
	group := keybindDefs[idx].Group
	var out []string
	for i, d := range keybindDefs {
		if i != idx && d.Group == group {
			out = append(out, d.Action)
		}
	}
	return out
}

// applyKeybind assigns key to the action at def index idx, updates the live
// keymap + persisted settings, and saves. If another action in the same group
// already uses that key, a warning is logged (the binding still takes effect —
// cross-group sharing is valid, and within-group the first switch match wins).
func (s *session) applyKeybind(idx int, key string) {
	if idx < 0 || idx >= len(keybindDefs) {
		return
	}
	action := keybindDefs[idx].Action
	for _, other := range keybindsInGroup(idx) {
		if s.keybinds[other] == key {
			s.logWarn(fmt.Sprintf("warning: %q is also bound to %q (same group: %s)",
				key, other, keybindDefs[idx].Group))
			break
		}
	}
	s.keybinds[action] = key
	if s.settings.Keybinds == nil {
		s.settings.Keybinds = map[string]string{}
	}
	s.settings.Keybinds[action] = key
	_ = s.settings.save()
}

// resetKeybind restores the action at def index idx to its default key and
// removes the user override from persisted settings.
func (s *session) resetKeybind(idx int) {
	if idx < 0 || idx >= len(keybindDefs) {
		return
	}
	d := keybindDefs[idx]
	s.keybinds[d.Action] = d.Default
	if s.settings.Keybinds != nil {
		delete(s.settings.Keybinds, d.Action)
		if len(s.settings.Keybinds) == 0 {
			s.settings.Keybinds = nil
		}
	}
	_ = s.settings.save()
}

// clearKeybind disables (unbinds) the action at def index idx: sets the live
// keymap entry to "" so kb() never matches it, and persists the empty string
// as a user override. This is how you remove an alternate key (e.g. clear
// ctrl+k so only ctrl+p opens the palette) or disable an action entirely.
func (s *session) clearKeybind(idx int) {
	if idx < 0 || idx >= len(keybindDefs) {
		return
	}
	action := keybindDefs[idx].Action
	s.keybinds[action] = ""
	if s.settings.Keybinds == nil {
		s.settings.Keybinds = map[string]string{}
	}
	s.settings.Keybinds[action] = ""
	_ = s.settings.save()
}

// captureKeybind assigns a key that arrived outside the normal KeyMsg path
// (currently only Ctrl+Enter, which arrives as an unrecognized CSI sequence)
// to the action selected in the keybinds modal's capture mode. Ctrl+Enter can
// only ever fire for the steer action (the CSI dispatch in Update() only routes
// it to steer), so binding it to anything else would be a silently-dead binding —
// reject that with a clear error.
func (s *session) captureKeybind(key string) {
	if s.modal.kind != modalKeybinds || !s.modal.editing {
		return
	}
	if key == "ctrl+enter" {
		idx := s.modal.cursor
		if idx < 0 || idx >= len(keybindDefs) || keybindDefs[idx].Action != "steer" {
			s.logError("ctrl+enter can only be bound to the steer action")
			s.modal.editing = false
			return
		}
	}
	if key == "shift+enter" {
		idx := s.modal.cursor
		if idx < 0 || idx >= len(keybindDefs) || keybindDefs[idx].Action != "newline" {
			s.logError("shift+enter can only be bound to the newline action")
			s.modal.editing = false
			return
		}
	}
	s.applyKeybind(s.modal.cursor, key)
	s.modal.editing = false
}

// renderKeybindsModal renders the /keybinds list with group headers, the
// current key for each action, and a capture-mode indicator on the selected row.
// The body is built into a flat line slice FIRST (including group headers +
// blank separators), then windowed by visual line so the scroll math accounts
// for the extra header lines and the cursor is always kept on-screen.
func (s *session) renderKeybindsModal() string {
	w := s.modalWidth(96)

	rowW := w - 4 // border(2) + padding(2)
	if rowW < 1 {
		rowW = 1
	}
	truncStyle := lipgloss.NewStyle().MaxWidth(rowW)
	hiStyle := lipgloss.NewStyle().
		Background(lipgloss.Color(c.dim)).
		Foreground(lipgloss.Color(c.fg)).
		Width(rowW)

	// Build the full body (post-title/separator) as a flat list of styled
	// lines, tracking the visual line index of each action row so we can
	// window around the cursor's true visual position.
	type row struct {
		text     string
		isAction bool
	}
	var body []row
	prevGroup := ""
	cursorLine := 0
	for i, d := range keybindDefs {
		if d.Group != prevGroup {
			if prevGroup != "" {
				body = append(body, row{text: ""})
			}
			body = append(body, row{text: accentStyle.Render("  " + d.Group)})
			prevGroup = d.Group
		}
		if i == s.modal.cursor {
			cursorLine = len(body)
		}
		marker := "  "
		if i == s.modal.cursor {
			marker = accentStyle.Render("▸ ")
		}
		key := s.keybinds[d.Action]
		if key == "" {
			key = d.Default
		}
		keyLabel := keybindLabel(key)
		descW := rowW - 2 - 22 - 16
		if descW < 0 {
			descW = 0
		}
		desc := truncateFit(d.Desc, descW)
		var rowStr string
		if s.modal.editing && i == s.modal.cursor {
			capture := "press a key…"
			rowStr = marker + baseStyle.Render(fmt.Sprintf("%-22s", d.Action)) + accentStyle.Render(capture) + strings.Repeat(" ", max(0, 16-runeLen(capture))) + dimStyle.Render(desc)
		} else {
			rowStr = marker + baseStyle.Render(fmt.Sprintf("%-22s", d.Action)) + accentStyle.Render(keyLabel) + strings.Repeat(" ", max(0, 16-runeLen(keyLabel))) + dimStyle.Render(desc)
		}
		rowStr = truncStyle.Render(rowStr)
		if i == s.modal.cursor {
			rowStr = hiStyle.Render(rowStr)
		}
		body = append(body, row{text: rowStr, isAction: true})
	}

	// Available height for body rows (after title + separator + footer + blank).
	maxVisible := s.height - 7
	if maxVisible < 1 {
		maxVisible = 1
	}
	total := len(body)
	// Window: keep the cursor's visual line on-screen.
	start := 0
	if total > maxVisible {
		start = cursorLine - maxVisible/2
		if start < 0 {
			start = 0
		}
		if start+maxVisible > total {
			start = total - maxVisible
			if start < 0 {
				start = 0
			}
		}
	}
	s.modal.scroll = start
	end := start + maxVisible
	if end > total {
		end = total
	}

	var lines []string
	lines = append(lines, accentStyle.Render("◆ Keybindings"))
	lines = append(lines, separatorStyle.Render(strings.Repeat("─", w-2)))
	for _, r := range body[start:end] {
		lines = append(lines, r.text)
	}
	if total > maxVisible {
		lines = append(lines, dimStyle.Render(fmt.Sprintf("  (%d more · ↑↓ scroll)", total-maxVisible)))
	}
	lines = append(lines, "")

	// Footer shows the live nav/close/select keys so the user knows how to
	// operate the modal with their current bindings.
	navKey := keybindLabel(s.keybinds["nav_up"])
	selectKey := keybindLabel(s.keybinds["select"])
	if s.modal.editing {
		lines = append(lines, truncStyle.Render(dimStyle.Render("  press a key to bind · esc cancels")))
	} else {
		footer := fmt.Sprintf("  %s/%s navigate · %s rebind · ⌫ reset · del clear · esc close",
			navKey, keybindLabel(s.keybinds["nav_down"]), selectKey)
		lines = append(lines, truncStyle.Render(dimStyle.Render(footer)))
	}
	bodyStr := strings.Join(lines, "\n")
	return modalBox(w, bodyStr)
}

// runeLen returns the number of runes in s (visible width for ASCII + BMP).
func runeLen(s string) int {
	return len([]rune(s))
}

// helpKeybindLines builds the keybinding section of the help text from the live
// keymap so /help always reflects the user's current bindings.
func (s *session) helpKeybindLines() []string {
	key := func(action string) string {
		return keybindLabel(s.keybinds[action])
	}
	return []string{
		"Keybindings",
		fmt.Sprintf("  %-18s open command palette", key("command_palette")+" / "+key("command_palette_alt")),
		fmt.Sprintf("  %-18s command palette (when input empty)", "/"),
		fmt.Sprintf("  %-18s toggle reasoning collapse", key("toggle_reasoning")),
		fmt.Sprintf("  %-18s expand / collapse last tool output", key("toggle_tool_output")),
		fmt.Sprintf("  %-18s set reasoning effort (per model)", key("reasoning_picker")),
		fmt.Sprintf("  %-18s quit", key("quit")),
		fmt.Sprintf("  %-18s close modal / deny / abort", key("close")),
		"",
		"Scrolling the transcript",
		fmt.Sprintf("  %-18s scroll a page up / down", key("scroll_page_up")+" / "+key("scroll_page_down")),
		fmt.Sprintf("  %-18s scroll a line up / down", key("scroll_line_up")+" / "+key("scroll_line_down")),
		fmt.Sprintf("  %-18s jump to top / bottom", key("scroll_top")+" / "+key("scroll_bottom")),
		"  (scrolling up pauses auto-follow; sending a",
		"   message or reaching the bottom re-pins)",
		"",
		"While a turn is running (in-flight)",
		fmt.Sprintf("  %-18s insert a line break (multi-line input)", key("newline")),
		fmt.Sprintf("  %-18s queue a follow-up message", key("send")),
		fmt.Sprintf("  %-18s steer (interrupt + redirect the model)", key("steer")),
		fmt.Sprintf("  %-18s drop queued, or abort if none queued", key("close")),
		"  /steer <msg>      steer (works on every terminal)",
		"",
		"Approval (when prompted)",
		fmt.Sprintf("  %-18s approve once", key("approve")),
		fmt.Sprintf("  %-18s approve & stop asking", key("approve_always")),
		fmt.Sprintf("  %-18s deny", key("deny")),
		"",
		"Navigation (modals, welcome, @-mention)",
		fmt.Sprintf("  %-18s move selection up / down", key("nav_up")+" / "+key("nav_down")),
		fmt.Sprintf("  %-18s select / confirm / accept", key("select")),
		fmt.Sprintf("  %-18s accept @-mention selection", key("mention_accept")),
		"",
		"Customize all keybindings: /keybinds",
	}
}
