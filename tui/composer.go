package main

import (
	"strings"

	"charm.land/bubbles/v2/key"
	"charm.land/bubbles/v2/textarea"
	"charm.land/lipgloss/v2"
)

// newComposer builds the main chat textarea. Enter is unbound so the session
// send keybind owns it; Shift+Enter / the newline keybind inserts via
// InsertRune. Custom inputContent still paints the bordered composer.
func newComposer() textarea.Model {
	ta := textarea.New()
	ta.Placeholder = "Chat with the agent…  (/ commands · ? help)"
	ta.Prompt = ""
	ta.ShowLineNumbers = false
	ta.CharLimit = 0
	ta.MaxHeight = 99
	ta.SetHeight(1)
	ta.SetVirtualCursor(false) // custom cursor in inputContent

	km := textarea.DefaultKeyMap()
	km.InsertNewline.Unbind() // Enter sends; newline keybind inserts explicitly
	ta.KeyMap = km

	st := ta.Styles()
	st.Focused.Placeholder = placeholderStyle
	st.Blurred.Placeholder = placeholderStyle
	st.Focused.Text = composerTextStyle()
	st.Blurred.Text = composerTextStyle()
	st.Focused.Base = lipgloss.NewStyle()
	st.Blurred.Base = lipgloss.NewStyle()
	st.Focused.CursorLine = lipgloss.NewStyle()
	st.Blurred.CursorLine = lipgloss.NewStyle()
	ta.SetStyles(st)
	ta.Focus()
	return ta
}

// refreshComposerStyles reapplies theme-derived styles after a theme switch.
func (s *session) refreshComposerStyles() {
	st := s.input.Styles()
	st.Focused.Placeholder = placeholderStyle
	st.Blurred.Placeholder = placeholderStyle
	st.Focused.Text = composerTextStyle()
	st.Blurred.Text = composerTextStyle()
	s.input.SetStyles(st)
}

// inputPosition returns the absolute rune offset of the textarea cursor,
// matching textinput.Position() so mention/history helpers stay unchanged.
func inputPosition(m textarea.Model) int {
	val := m.Value()
	parts := strings.Split(val, "\n")
	row := m.Line()
	if row < 0 {
		row = 0
	}
	if len(parts) == 0 {
		return m.Column()
	}
	if row >= len(parts) {
		row = len(parts) - 1
	}
	pos := 0
	for i := 0; i < row; i++ {
		pos += len([]rune(parts[i])) + 1
	}
	return pos + m.Column()
}

// setInputCursor places the textarea cursor at an absolute rune offset.
func setInputCursor(m *textarea.Model, pos int) {
	val := m.Value()
	r := []rune(val)
	if pos < 0 {
		pos = 0
	}
	if pos > len(r) {
		pos = len(r)
	}
	before := string(r[:pos])
	row := strings.Count(before, "\n")
	col := len([]rune(before))
	if i := strings.LastIndex(before, "\n"); i >= 0 {
		col = len([]rune(before[i+1:]))
	}
	m.MoveToBegin()
	for i := 0; i < row; i++ {
		m.CursorDown()
	}
	m.SetCursorColumn(col)
}

// bindingFor returns a bubbles/key Binding for a named action using the
// session's effective keymap (for help.Model short views).
func (s *session) bindingFor(action, desc string) key.Binding {
	k := s.keyHint(action)
	if k == "" {
		return key.NewBinding(key.WithDisabled())
	}
	return key.NewBinding(key.WithKeys(k), key.WithHelp(k, desc))
}
