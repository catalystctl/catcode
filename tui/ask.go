package main

import (
	"encoding/json"
	"fmt"
	"strings"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

// askPrompt is the TUI state for a pending `ask` tool call. The core emits an
// `ask_request` event and blocks until `ask_reply` arrives; this flyout renders
// each question with its own control (a horizontal select cycle or a free-text
// box) and sends the answers back on submit, or null on skip.
type askPrompt struct {
	requestID string
	questions []askQuestion
	focusIdx  int
	// errMsg is a transient inline error (e.g. "Required: …") shown in the
	// flyout when submit fails validation. Cleared on the next non-submit
	// keypress so it never accumulates in the transcript (the old behavior
	// logged a fresh "✗ required" line per Enter, spamming the log).
	errMsg string
}

// askQuestion is one field in the flyout.
type askQuestion struct {
	id          string
	prompt      string
	qtype       string // "select" | "text"
	options     []string
	allowCustom bool
	required    bool
	placeholder string
	// select: current option index. When allowCustom is true, index len(options)
	// is the "Custom…" entry (custom free-text via `input`).
	selIdx int
	// text questions (and select-in-custom-mode) use this input.
	input textinput.Model
}

// isCustom reports whether a select question is in custom free-text mode.
func (q *askQuestion) isCustom() bool {
	return q.qtype == "select" && q.allowCustom && q.selIdx == len(q.options)
}

// currentValue returns the answer string for this question ("" if unanswered).
func (q *askQuestion) currentValue() string {
	if q.qtype == "text" || q.isCustom() {
		return strings.TrimSpace(q.input.Value())
	}
	if q.selIdx >= 0 && q.selIdx < len(q.options) {
		return q.options[q.selIdx]
	}
	return ""
}

// numSelectSlots returns the cycle length for a select question (options + the
// "Custom…" pseudo-entry when allowCustom).
func (q *askQuestion) numSelectSlots() int {
	if q.qtype != "select" {
		return 0
	}
	n := len(q.options)
	if q.allowCustom {
		n++
	}
	return n
}

// parseAskRequest builds an askPrompt from the `ask_request` event's questions
// JSON array, creating a textinput for each text-capable question.
func parseAskRequest(requestID string, raw json.RawMessage) *askPrompt {
	var qs []map[string]json.RawMessage
	if err := json.Unmarshal(raw, &qs); err != nil || len(qs) == 0 {
		return nil
	}
	out := &askPrompt{requestID: requestID, questions: []askQuestion{}}
	for _, qm := range qs {
		gs := func(k string) string {
			if v, ok := qm[k]; ok {
				var s string
				if json.Unmarshal(v, &s) == nil {
					return s
				}
			}
			return ""
		}
		gb := func(k string, def bool) bool {
			if v, ok := qm[k]; ok {
				var b bool
				if json.Unmarshal(v, &b) == nil {
					return b
				}
			}
			return def
		}
		ga := func(k string) []string {
			if v, ok := qm[k]; ok {
				var a []string
				if json.Unmarshal(v, &a) == nil {
					return a
				}
			}
			return nil
		}
		q := askQuestion{
			id:          gs("id"),
			prompt:      gs("prompt"),
			qtype:       gs("type"),
			options:     ga("options"),
			allowCustom: gb("allowCustom", false),
			required:    gb("required", true),
			placeholder: gs("placeholder"),
		}
		ti := textinput.New()
		ti.Prompt = ""
		ti.Placeholder = q.placeholder
		if ti.Placeholder == "" {
			if q.qtype == "text" {
				ti.Placeholder = "Type your answer…"
			} else {
				ti.Placeholder = "Type a custom answer…"
			}
		}
		// textinput v2 dropped the public PlaceholderStyle field; the placeholder
		// style now lives on Styles().{Focused,Blurred}.Placeholder.
		st := ti.Styles()
		st.Focused.Placeholder = placeholderStyle
		st.Blurred.Placeholder = placeholderStyle
		ti.SetStyles(st)
		q.input = ti
		out.questions = append(out.questions, q)
	}
	// Focus the first text-capable question's input so typing works immediately
	// when the first question is free-text.
	out.focusInput()
	return out
}

// focusInput focuses the current question's textinput (text or custom select),
// blurring all others. Select questions (not custom) need no text focus.
func (a *askPrompt) focusInput() {
	for i := range a.questions {
		if i == a.focusIdx && (a.questions[i].qtype == "text" || a.questions[i].isCustom()) {
			a.questions[i].input.Focus()
		} else {
			a.questions[i].input.Blur()
		}
	}
}

// answers builds the {id: answer} object for ask_reply, including only
// non-empty answers. Returns (obj, missingRequired) where missingRequired lists
// the prompts of required questions left empty.
func (a *askPrompt) answers() (map[string]string, []string) {
	obj := map[string]string{}
	var missing []string
	for _, q := range a.questions {
		v := q.currentValue()
		if v == "" {
			if q.required {
				missing = append(missing, q.prompt)
			}
			continue
		}
		obj[q.id] = v
	}
	return obj, missing
}

// sendReply dispatches the ask_reply command and clears the flyout.
func (s *session) sendAskReply(a *askPrompt, answers any) {
	s.sendCore(map[string]any{
		"type":       "ask_reply",
		"request_id": a.requestID,
		"answers":    answers,
	})
	s.pendingAsk = nil
	s.input.Focus()
	s.layout()
}

// handleAskKey owns all keys while the ask flyout is open.
func (s *session) handleAskKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	a := s.pendingAsk
	if a == nil {
		return s, nil
	}
	// Any key other than submit clears a stale inline error so it doesn't
	// linger after the user starts fixing the field.
	if !s.kb(msg, "send") {
		a.errMsg = ""
	}
	// Esc / close: skip the whole prompt (send null).
	if s.kb(msg, "close") {
		s.sendAskReply(a, nil)
		return s, nil
	}
	// Enter: submit (validate required first).
	if s.kb(msg, "send") {
		obj, missing := a.answers()
		if len(missing) > 0 {
			// Show the error INLINE in the flyout (transient) instead of
			// logging to the transcript — repeated Enter on an empty
			// required field used to spam "✗ required" lines.
			a.errMsg = fmt.Sprintf("Required: %s", strings.Join(missing, "; "))
			return s, nil
		}
		s.sendAskReply(a, obj)
		s.logSuccess("↦ answers sent")
		return s, nil
	}
	// Tab / ↓ / j: next question (clamp at last). The bare "down" fallback
	// mirrors the scroll handler so arrows always navigate even if a user
	// disabled/rebound nav_down in /keybinds.
	if s.kb(msg, "field_next") || msg.String() == "down" || s.kbAny(msg, "nav_down", "nav_down_alt") {
		if a.focusIdx < len(a.questions)-1 {
			a.focusIdx++
			a.focusInput()
		}
		return s, nil
	}
	// Shift+Tab / ↑ / k: previous question (clamp at first).
	if s.kb(msg, "field_prev") || msg.String() == "up" || s.kbAny(msg, "nav_up", "nav_up_alt") {
		if a.focusIdx > 0 {
			a.focusIdx--
			a.focusInput()
		}
		return s, nil
	}
	// Route to the focused question's control.
	q := &a.questions[a.focusIdx]
	if q.qtype == "select" && !q.isCustom() {
		// ←/→ or h/l cycle the options (incl. the "Custom…" slot).
		if s.kbAny(msg, "cycle_left", "cycle_left_alt") || msg.String() == "left" || msg.String() == "h" {
			n := q.numSelectSlots()
			if n > 0 {
				q.selIdx = (q.selIdx - 1 + n) % n
				a.focusInput()
			}
			return s, nil
		}
		if s.kbAny(msg, "cycle_right", "cycle_right_alt") || msg.String() == "right" || msg.String() == "l" {
			n := q.numSelectSlots()
			if n > 0 {
				q.selIdx = (q.selIdx + 1) % n
				a.focusInput()
			}
			return s, nil
		}
		// Any other key on a select (not custom): ignore (no typing).
		return s, nil
	}
	// Text question or select-in-custom-mode: route to the textinput.
	var cmd tea.Cmd
	q.input, cmd = q.input.Update(msg)
	return s, cmd
}

// renderAskOverlay renders the ask flyout as a centered modal over the base view.
func (s *session) renderAskOverlay(base string) string {
	if s.pendingAsk == nil {
		return base
	}
	box := s.renderAskBox()
	w := s.width
	h := s.height
	if bh := lipgloss.Height(box); bh > h && h > 0 {
		ls := strings.Split(box, "\n")
		if h <= len(ls) {
			box = strings.Join(ls[:h], "\n")
		}
	}
	return lipgloss.Place(w, h, lipgloss.Center, lipgloss.Center, box)
}

// renderAskBox builds the flyout body.
func (s *session) renderAskBox() string {
	a := s.pendingAsk
	boxW := s.width - 8
	if s.width < 44 {
		boxW = s.width
	}
	if boxW > 74 {
		boxW = 74
	}
	if boxW < 1 {
		boxW = 1
	}
	inner := boxW - 6 // border(2) + horizontal padding(4)
	if inner < 1 {
		inner = 1
	}

	var b strings.Builder
	title := accentStyle.Render(truncate("❓ Answer the questions", inner))
	b.WriteString(title + "\n\n")

	for i, q := range a.questions {
		focused := i == a.focusIdx
		marker := "◇"
		promptStyle := dimStyle
		if focused {
			marker = "❯"
			promptStyle = boldBaseStyle
		}
		req := ""
		if q.required {
			req = mutedStyle.Render(" *")
		}
		b.WriteString(fmt.Sprintf("%s %s%s\n", accentStyle.Render(marker), promptStyle.Render(truncate(q.prompt, max(1, inner-4))), req))

		if q.qtype == "select" && !q.isCustom() {
			// Horizontal cycle: ◀  Option  ▶  [i/n]
			cur := "—"
			if q.selIdx >= 0 && q.selIdx < len(q.options) {
				cur = q.options[q.selIdx]
			} else if q.isCustom() {
				cur = "Custom…"
			}
			n := q.numSelectSlots()
			pos := q.selIdx + 1
			if q.isCustom() {
				pos = n
			}
			opt := accentStyle.Render("◀") + "  " + boldBaseStyle.Render(truncate(cur, max(1, inner-16))) + "  " + accentStyle.Render("▶")
			cnt := mutedStyle.Render(fmt.Sprintf("[%d/%d]", pos, n))
			line := "    " + opt
			pad := inner - lipgloss.Width(opt) - lipgloss.Width(cnt) - 4
			if pad < 0 {
				pad = 0
			}
			line += strings.Repeat(" ", pad) + cnt
			b.WriteString(line + "\n")
			hint := mutedStyle.Render("←/→ or h/l to choose")
			if q.allowCustom {
				hint = mutedStyle.Render("←/→ · cycle to “Custom…” to type your own")
			}
			if inner >= 28 {
				b.WriteString("    " + hint + "\n")
			}
		} else {
			// Text input (text question, or select-in-custom-mode).
			q.input.SetWidth(max(1, inner-8))
			if q.isCustom() {
				b.WriteString("    " + mutedStyle.Render("Custom answer:") + "\n")
			}
			field := "    " + q.input.View()
			b.WriteString(field + "\n")
		}
		if i < len(a.questions)-1 {
			b.WriteString("\n")
		}
	}

	b.WriteString("\n")
	sendKey, closeKey := s.keyHint("send"), s.keyHint("close")
	if sendKey == "" {
		sendKey = "unbound"
	}
	if closeKey == "" {
		closeKey = "unbound"
	}
	footerText := fmt.Sprintf("[Tab/↑↓] navigate · [%s] submit · [%s] skip", sendKey, closeKey)
	if a.focusIdx < len(a.questions) {
		q := a.questions[a.focusIdx]
		if q.qtype == "select" && !q.isCustom() {
			footerText = fmt.Sprintf("[←/→] choose · [Tab/↑↓] navigate · [%s] submit · [%s] skip", sendKey, closeKey)
		}
	}
	footer := mutedStyle.Render(truncate(footerText, inner))
	b.WriteString(footer)
	if a.errMsg != "" {
		b.WriteString("\n" + errStyle.Render(truncate("✗ "+a.errMsg, inner)))
	}

	bodyLines := strings.Split(b.String(), "\n")
	focusLine := 0
	for i, line := range bodyLines {
		if strings.Contains(line, "❯") {
			focusLine = i
		}
	}
	// Rounded border + vertical padding consume four rows. Preserve title and
	// footer while centering the active question in the remaining viewport.
	tail := 2
	if a.errMsg != "" {
		tail = 3
	}
	bodyLines = focusWindow(bodyLines, focusLine, s.height-4, 2, tail)
	clip := lipgloss.NewStyle().MaxWidth(inner)
	for i := range bodyLines {
		bodyLines[i] = clip.Render(bodyLines[i])
	}
	body := strings.Join(bodyLines, "\n")
	return lipgloss.NewStyle().
		Width(boxW).
		Padding(1, 2).
		Border(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(c.accent)).
		Render(body)
}
