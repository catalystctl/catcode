package main

import (
	"encoding/json"
	"fmt"
	"strings"

	tea "charm.land/bubbletea/v2"
	"charm.land/huh/v2"
	"charm.land/lipgloss/v2"
)

const askCustomSentinel = "__custom__"

// askPrompt is the TUI state for a pending `ask` tool call. The core emits an
// `ask_request` event and blocks until `ask_reply` arrives; this flyout renders
// each question via a huh form and sends the answers back on submit, or null on skip.
type askPrompt struct {
	requestID    string
	questions    []askQuestion
	fieldValues  []string // parallel values bound to form fields (per question)
	customValues []string // custom text when select picks "__custom__"
	form         *huh.Form
	focusIdx     int // current question index (synced from form focus)
	errMsg       string
}

// askQuestion is one field in the flyout (metadata for validation/navigation).
type askQuestion struct {
	id          string
	prompt      string
	qtype       string // "select" | "text"
	options     []string
	allowCustom bool
	required    bool
	placeholder string
}

// parseAskRequest builds an askPrompt from the `ask_request` event's questions
// JSON array, wiring a huh form with Select/Input fields per question.
func parseAskRequest(requestID string, raw json.RawMessage) *askPrompt {
	var qs []map[string]json.RawMessage
	if err := json.Unmarshal(raw, &qs); err != nil || len(qs) == 0 {
		return nil
	}
	out := &askPrompt{requestID: requestID, questions: []askQuestion{}}
	var fields []huh.Field
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
		out.questions = append(out.questions, q)
		qIdx := len(out.questions) - 1

		switch q.qtype {
		case "select":
			val := ""
			if len(q.options) > 0 {
				val = q.options[0]
			}
			out.fieldValues = append(out.fieldValues, val)
			out.customValues = append(out.customValues, "")

			opts := make([]huh.Option[string], len(q.options))
			for i, o := range q.options {
				opts[i] = huh.NewOption(o, o)
			}
			if q.allowCustom {
				opts = append(opts, huh.NewOption("Custom…", askCustomSentinel))
			}
			// Inline renders the select as a single-line "‹ option ›" picker.
			// Required for key handling, not just looks: huh enables Left/Right
			// (and disables Up/Down) on a select only when inline, so ←/→ cycle
			// the option natively. With a non-inline select Left/Right are disabled,
			// and mutating the bound value can never move huh's private `selected`
			// cursor that the View renders from — so the display stayed stuck on
			// the first option and the user could never pick another.
			sel := huh.NewSelect[string]().
				Title(q.prompt).
				Key(q.id).
				Inline(true).
				Options(opts...).
				Value(&out.fieldValues[qIdx])
			if q.required {
				sel.Validate(func(v string) error {
					if v == "" {
						return fmt.Errorf("required")
					}
					return nil
				})
			}
			fields = append(fields, sel)
			if q.allowCustom {
				ph := q.placeholder
				if ph == "" {
					ph = "Type a custom answer…"
				}
				cust := huh.NewInput().
					Title("Custom answer").
					Key(q.id + "_custom").
					Placeholder(ph).
					Value(&out.customValues[qIdx])
				if q.required {
					cust.Validate(func(v string) error {
						if out.fieldValues[qIdx] == askCustomSentinel && strings.TrimSpace(v) == "" {
							return fmt.Errorf("required")
						}
						return nil
					})
				}
				fields = append(fields, cust)
			}
		default: // text
			out.fieldValues = append(out.fieldValues, "")
			out.customValues = append(out.customValues, "")
			ph := q.placeholder
			if ph == "" {
				ph = "Type your answer…"
			}
			inp := huh.NewInput().
				Title(q.prompt).
				Key(q.id).
				Placeholder(ph).
				Value(&out.fieldValues[qIdx])
			if q.required {
				inp.Validate(func(v string) error {
					if strings.TrimSpace(v) == "" {
						return fmt.Errorf("required")
					}
					return nil
				})
			}
			fields = append(fields, inp)
		}
	}

	form := huh.NewForm(huh.NewGroup(fields...)).
		WithTheme(catalystHuhTheme()).
		WithShowHelp(true).
		WithShowErrors(false)
	out.form = form
	out.focusIdx = 0
	_ = form.Init()
	return out
}

func (a *askPrompt) focusedQuestionIndex() int {
	if a.form == nil {
		return a.focusIdx
	}
	f := a.form.GetFocusedField()
	if f == nil {
		return a.focusIdx
	}
	key := f.GetKey()
	for i, q := range a.questions {
		if key == q.id || key == q.id+"_custom" {
			return i
		}
	}
	return a.focusIdx
}

func (a *askPrompt) syncFocusFromForm() {
	if a.form == nil {
		return
	}
	a.focusIdx = a.focusedQuestionIndex()
}

// focusedOnCustom reports whether form focus is on this question's custom
// text input (the `_custom` field), used to route keys between the inline
// picker and the custom-answer box.
func (a *askPrompt) focusedOnCustom() bool {
	if a.form == nil || a.focusIdx < 0 || a.focusIdx >= len(a.questions) {
		return false
	}
	f := a.form.GetFocusedField()
	if f == nil {
		return false
	}
	return f.GetKey() == a.questions[a.focusIdx].id+"_custom"
}

// advanceField moves form focus by one field (NextField/PrevField) and keeps
// focusIdx in sync. Used to step between a select and its `_custom` input.
func (a *askPrompt) advanceField(delta int) {
	if a.form == nil {
		return
	}
	var m huh.Model
	var cmd tea.Cmd
	if delta >= 0 {
		m, cmd = a.form.Update(huh.NextField())
	} else {
		m, cmd = a.form.Update(huh.PrevField())
	}
	if f, ok := m.(*huh.Form); ok {
		a.form = f
	}
	_ = cmd
	a.syncFocusFromForm()
}

// isAskCycleKey reports whether msg is a select-cycling key (←/→/h/l or
// their rebound equivalents).
func (s *session) isAskCycleKey(msg tea.KeyPressMsg) bool {
	return s.kbAny(msg, "cycle_left", "cycle_left_alt", "cycle_right", "cycle_right_alt") ||
		msg.String() == "left" || msg.String() == "right" || msg.String() == "h" || msg.String() == "l"
}

// jumpToQuestion moves form focus to the primary field of target (by question
// index). Select+allowCustom inserts an extra huh field, so we cannot step by
// question-count alone — advance Next/PrevField until the focused question matches.
func (a *askPrompt) jumpToQuestion(target int) {
	if a.form == nil || target < 0 || target >= len(a.questions) {
		return
	}
	forward := target >= a.focusedQuestionIndex()
	for guard := 0; guard < 64; guard++ {
		cur := a.focusedQuestionIndex()
		var curKey string
		if f := a.form.GetFocusedField(); f != nil {
			curKey = f.GetKey()
		}
		if cur == target {
			break
		}
		var m huh.Model = a.form
		var cmd tea.Cmd
		if forward {
			m, cmd = a.form.Update(huh.NextField())
		} else {
			m, cmd = a.form.Update(huh.PrevField())
		}
		if f, ok := m.(*huh.Form); ok {
			a.form = f
		}
		_ = cmd
		nextKey := ""
		if f := a.form.GetFocusedField(); f != nil {
			nextKey = f.GetKey()
		}
		if nextKey == curKey {
			break // focus did not move
		}
	}
	// Prefer landing on the question's primary field (not _custom) so ←/→
	// select cycling still works after Tab/↑↓ navigation.
	if f := a.form.GetFocusedField(); f != nil && f.GetKey() == a.questions[target].id+"_custom" {
		m, cmd := a.form.Update(huh.PrevField())
		if form, ok := m.(*huh.Form); ok {
			a.form = form
		}
		_ = cmd
	}
	a.focusIdx = a.focusedQuestionIndex()
}

// answers builds the {id: answer} object for ask_reply, including only
// non-empty answers. Returns (obj, missingRequired) where missingRequired lists
// the prompts of required questions left empty.
func (a *askPrompt) answers() (map[string]string, []string) {
	obj := map[string]string{}
	var missing []string
	for i, q := range a.questions {
		var v string
		switch q.qtype {
		case "select":
			sel := a.fieldValues[i]
			if sel == askCustomSentinel {
				v = strings.TrimSpace(a.customValues[i])
			} else {
				v = sel
			}
		default:
			v = strings.TrimSpace(a.fieldValues[i])
		}
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

func (s *session) pumpHuhAsk(msg tea.Msg) {
	a := s.pendingAsk
	if a == nil || a.form == nil {
		return
	}
	m, cmd := a.form.Update(msg)
	if f, ok := m.(*huh.Form); ok {
		a.form = f
	}
	for i := 0; i < 8 && cmd != nil; i++ {
		next := cmd()
		if next == nil {
			break
		}
		m, cmd = a.form.Update(next)
		if f, ok := m.(*huh.Form); ok {
			a.form = f
		}
	}
	a.syncFocusFromForm()
}

// handleAskKey owns all keys while the ask flyout is open.
func (s *session) handleAskKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	a := s.pendingAsk
	if a == nil {
		return s, nil
	}
	if !s.kb(msg, "send") {
		a.errMsg = ""
	}
	if s.kb(msg, "close") {
		s.sendAskReply(a, nil)
		return s, nil
	}
	if s.kb(msg, "send") {
		obj, missing := a.answers()
		if len(missing) > 0 {
			a.errMsg = fmt.Sprintf("Required: %s", strings.Join(missing, "; "))
			return s, nil
		}
		s.sendAskReply(a, obj)
		s.logSuccess("↦ answers sent")
		return s, nil
	}
	if s.kb(msg, "field_next") || msg.String() == "down" || s.kbAny(msg, "nav_down", "nav_down_alt") {
		if a.focusIdx < len(a.questions)-1 {
			a.jumpToQuestion(a.focusIdx + 1)
		}
		return s, nil
	}
	if s.kb(msg, "field_prev") || msg.String() == "up" || s.kbAny(msg, "nav_up", "nav_up_alt") {
		if a.focusIdx > 0 {
			a.jumpToQuestion(a.focusIdx - 1)
		}
		return s, nil
	}
	q := &a.questions[a.focusIdx]
	onCustomInput := a.focusedOnCustom()
	// Select questions render as a single-line inline picker (‹ option ›).
	// ←/→/h/l cycle it: forward to huh so its internal `selected` cursor —
	// the View's source of truth — moves and the bound value syncs. The old
	// cycleSelect mutated only the bound value, which huh never re-reads, so
	// the cursor (and display) stayed stuck on the first option and no other
	// option could be picked.
	if q.qtype == "select" {
		if s.isAskCycleKey(msg) {
			if onCustomInput {
				// ←/→ on the custom text input exits custom mode and returns
				// focus to the picker (stepping its cursor off "Custom…").
				a.advanceField(-1)
				s.pumpHuhAsk(msg)
				return s, nil
			}
			s.pumpHuhAsk(msg)
			// Landing on "Custom…" (allowCustom) enters its text input so the
			// user can type a value instead of picking a listed option.
			if q.allowCustom && a.fieldValues[a.focusIdx] == askCustomSentinel {
				a.advanceField(1)
			}
			return s, nil
		}
		// Swallow stray keys on the picker so letters don't start huh's filter
		// mode; they're meaningless for a select. (Custom input falls through.)
		if !onCustomInput {
			return s, nil
		}
	}
	// Text question, or the custom text input: forward typing/edits to huh.
	s.pumpHuhAsk(msg)
	return s, nil
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

// renderAskBox builds the flyout body from the huh form.
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
	inner := boxW - 6
	if inner < 1 {
		inner = 1
	}

	a.form.WithWidth(inner)
	formH := s.height - 10
	if formH < 4 {
		formH = 4
	}
	a.form.WithHeight(formH)
	body := a.form.View()
	bodyLines := strings.Split(body, "\n")
	focusLine := 0
	for i, line := range bodyLines {
		if strings.Contains(line, "┃") {
			focusLine = i
		}
	}
	if a.focusIdx < len(a.questions) {
		qtitle := a.questions[a.focusIdx].prompt
		for i, line := range bodyLines {
			if strings.Contains(stripANSI(line), qtitle) {
				focusLine = i
			}
		}
	}
	maxBody := s.height - 8
	if maxBody < 2 {
		maxBody = 2
	}
	if len(bodyLines) > maxBody {
		bodyLines = focusWindow(bodyLines, focusLine, maxBody, 1, 1)
		body = strings.Join(bodyLines, "\n")
	}

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
		if q.qtype == "select" && a.fieldValues[a.focusIdx] != askCustomSentinel {
			footerText = fmt.Sprintf("[←/→] choose · [Tab/↑↓] navigate · [%s] submit · [%s] skip", sendKey, closeKey)
		}
	}
	footer := mutedStyle.Render(truncate(footerText, inner))
	titleLine := accentStyle.Render(truncate("❓ Answer the questions", inner))
	focusHint := ""
	if s.height <= 12 && a.focusIdx >= 0 && a.focusIdx < len(a.questions) {
		focusHint = boldBaseStyle.Render(truncate(a.questions[a.focusIdx].prompt, inner)) + "\n"
	}
	panel := titleLine + "\n\n" + focusHint + body + "\n" + footer
	if a.errMsg != "" {
		panel += "\n" + errStyle.Render(truncate("✗ "+a.errMsg, inner))
	}

	box := lipgloss.NewStyle().
		Width(boxW).
		Padding(0, 2).
		Border(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(c.accent)).
		Render(panel)
	if s.height > 0 {
		lines := strings.Split(box, "\n")
		maxH := s.height - 2
		if maxH < 1 {
			maxH = 1
		}
		if len(lines) > maxH {
			box = strings.Join(lines[:maxH], "\n")
		}
	}
	return box
}
