package main

import (
	"fmt"
	"strings"
	"time"

	"charm.land/bubbles/v2/textinput"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

// sudoTimeoutMsg fires 30s after a sudo_request opens the flyout. If the user
// hasn't answered (the flyout is still open with the same request_id), the
// request auto-declines so the agent isn't blocked forever.
type sudoTimeoutMsg struct{ requestID string }

// sudoAutoClose is how long the sudo flyout stays open before auto-declining.
const sudoAutoClose = 30 * time.Second

// sudoTimeoutCmd returns a tea.Cmd that fires sudoTimeoutMsg after sudoAutoClose.
func sudoTimeoutCmd(requestID string) tea.Cmd {
	return tea.Tick(sudoAutoClose, func(time.Time) tea.Msg {
		return sudoTimeoutMsg{requestID: requestID}
	})
}

// sudoPrompt is the TUI state for a pending sudo_request: the agent wants to
// run a bash command that invokes `sudo`, and the core blocks until the user
// approves (with a password) or declines (Esc). The password is fed to
// `sudo -S` on stdin so sudo never touches /dev/tty and garbles the TUI.
type sudoPrompt struct {
	requestID string
	command   string
	input     textinput.Model
	// openedAt is when the flyout opened, for the auto-close countdown.
	openedAt time.Time
	// errMsg is a transient inline error shown in the flyout (cleared on next
	// non-submit keypress). Never logged to the transcript (avoids spam).
	errMsg string
}

// newSudoPrompt builds a sudoPrompt from the sudo_request event payload.
func newSudoPrompt(requestID, command string) *sudoPrompt {
	ti := textinput.New()
	ti.Prompt = ""
	ti.Placeholder = "Enter your sudo password…"
	ti.EchoMode = textinput.EchoPassword // mask: show dots, not the password
	st := ti.Styles()
	st.Focused.Placeholder = placeholderStyle
	st.Blurred.Placeholder = placeholderStyle
	ti.SetStyles(st)
	ti.Focus()
	return &sudoPrompt{
		requestID: requestID,
		command:   command,
		openedAt:  time.Now(),
		input:     ti,
	}
}

// sendSudoReply dispatches the sudo_reply command and clears the flyout.
func (s *session) sendSudoReply(p *sudoPrompt, approved bool) {
	pw := p.input.Value()
	s.sendCore(map[string]any{
		"type":       "sudo_reply",
		"request_id": p.requestID,
		"approved":   approved,
		"password":   pw,
	})
	s.pendingSudo = nil
	s.input.Focus()
	s.layout()
}

// handleSudoKey owns all keys while the sudo flyout is open.
func (s *session) handleSudoKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	p := s.pendingSudo
	if p == nil {
		return s, nil
	}
	// Any non-submit key clears a stale inline error.
	if !s.kb(msg, "send") && !s.kb(msg, "close") {
		p.errMsg = ""
	}
	// Esc / close: decline the sudo request (command NOT run).
	if s.kb(msg, "close") {
		s.sendSudoReply(p, false)
		s.logInfo("⊘ sudo request declined")
		return s, nil
	}
	// Enter: approve + send the password.
	if s.kb(msg, "send") {
		if strings.TrimSpace(p.input.Value()) == "" {
			closeKey := s.keyHint("close")
			if closeKey == "" {
				closeKey = "the decline key"
			}
			p.errMsg = "Password is required — type it, or press " + closeKey + " to decline"
			return s, nil
		}
		s.sendSudoReply(p, true)
		s.logSuccess("🔓 sudo approved — running command")
		return s, nil
	}
	// Route all other keys to the password textinput.
	var cmd tea.Cmd
	p.input, cmd = p.input.Update(msg)
	return s, cmd
}

// renderSudoOverlay renders the sudo flyout as a centered modal over the base
// view. No-op (returns base unchanged) when nothing is pending.
func (s *session) renderSudoOverlay(base string) string {
	if s.pendingSudo == nil {
		return base
	}
	box := s.renderSudoBox()
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

// renderSudoBox builds the flyout body.
func (s *session) renderSudoBox() string {
	p := s.pendingSudo
	boxW := s.width - 8
	if s.width < 48 {
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
	title := warnStyle.Render(truncate("🔐 Sudo command requested", inner))
	b.WriteString(title + "\n\n")

	b.WriteString(mutedStyle.Render(truncate("The agent wants to run a command that needs sudo:", inner)) + "\n")
	b.WriteString(codeTextStyle.Render(truncate(p.command, max(1, inner-2))) + "\n\n")

	b.WriteString(mutedStyle.Render(truncate("Enter your sudo password to approve:", inner)) + "\n")
	p.input.SetWidth(max(1, inner-8))
	b.WriteString("    " + p.input.View() + "\n")

	b.WriteString("\n")
	// Auto-close countdown: shows remaining seconds (updates each tickMsg).
	remaining := int(sudoAutoClose.Seconds() - time.Since(p.openedAt).Seconds())
	if remaining < 0 {
		remaining = 0
	}
	sendKey, closeKey := s.keyHint("send"), s.keyHint("close")
	if sendKey == "" {
		sendKey = "unbound"
	}
	if closeKey == "" {
		closeKey = "unbound"
	}
	footer := fmt.Sprintf("[%s] approve · [%s] decline · auto-close %ds", sendKey, closeKey, remaining)
	b.WriteString(mutedStyle.Render(truncate(footer, inner)))
	if p.errMsg != "" {
		b.WriteString("\n" + errStyle.Render(truncate("✗ "+p.errMsg, inner)))
	}

	bodyLines := strings.Split(b.String(), "\n")
	// Keep the password control and actions visible on very short terminals.
	focusLine := 0
	for i, line := range bodyLines {
		if strings.Contains(line, p.input.View()) {
			focusLine = i
			break
		}
	}
	tail := 2
	if p.errMsg != "" {
		tail = 3
	}
	bodyLines = focusWindow(bodyLines, focusLine, s.height-4, 1, tail)
	clip := lipgloss.NewStyle().MaxWidth(inner)
	for i := range bodyLines {
		bodyLines[i] = clip.Render(bodyLines[i])
	}
	body := strings.Join(bodyLines, "\n")
	return lipgloss.NewStyle().
		Width(boxW).
		Padding(1, 2).
		Border(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(c.warn)).
		Render(body)
}
