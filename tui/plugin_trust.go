package main

import (
	"encoding/json"
	"fmt"
	"strings"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

// Plugin trust prompt: project-scoped plugins (`.catalyst-code/plugins/*`
// shipped by the repo) are gated off until the user decides to trust or deny
// each one. The core emits `plugin_trust_prompt` on startup when there are
// undecided plugins (auto-appears once — decisions are persisted) and again on
// `/plugin-trust`. This modal lets the user cycle each plugin through
// undecided → trust → deny and apply the result; the core records the
// decisions and loads trusted plugins immediately.
type pluginTrustEntry struct {
	Name        string `json:"name"`
	Version     string `json:"version"`
	Description string `json:"description"`
	Path        string `json:"path"`
	Decision    string `json:"decision"` // "" | "trust" | "deny"
}

type pluginTrustPrompt struct {
	entries   []pluginTrustEntry
	decisions map[string]string // working state: name → "" | "trust" | "deny"
}

// parsePluginTrustPrompt parses a `plugin_trust_prompt` event payload
// (the `plugins` array).
func parsePluginTrustPrompt(raw json.RawMessage) []pluginTrustEntry {
	var entries []pluginTrustEntry
	if err := json.Unmarshal(raw, &entries); err != nil {
		return nil
	}
	return entries
}

// openPluginTrustModal opens the trust modal with the given entries (usually
// from a `plugin_trust_prompt` event).
func (s *session) openPluginTrustModal(entries []pluginTrustEntry) {
	decisions := make(map[string]string, len(entries))
	for _, e := range entries {
		decisions[e.Name] = e.Decision
	}
	s.pluginTrust = &pluginTrustPrompt{entries: entries, decisions: decisions}
	s.modal = newModal()
	s.modal.kind = modalPluginTrust
	s.modal.cursor = 0
}

// pluginTrustRows returns the number of selectable rows: one per plugin entry
// plus the trailing "Apply & close" row.
func (s *session) pluginTrustRows() int {
	if s.pluginTrust == nil {
		return 0
	}
	return len(s.pluginTrust.entries) + 1
}

// cyclePluginTrustDecision advances undecided → trust → deny → undecided for
// the focused plugin row.
func (s *session) cyclePluginTrustDecision(row int) {
	p := s.pluginTrust
	if p == nil || row < 0 || row >= len(p.entries) {
		return
	}
	name := p.entries[row].Name
	next := map[string]string{
		"":      "trust",
		"trust": "deny",
		"deny":  "",
	}
	p.decisions[name] = next[p.decisions[name]]
}

// applyPluginTrust sends the current decisions to the core and closes the
// modal. Only entries with a decided value are sent; undecided ones are left
// alone (they will re-prompt on the next load).
func (s *session) applyPluginTrust() {
	p := s.pluginTrust
	if p == nil {
		s.closeModal()
		return
	}
	decisions := map[string]string{}
	for name, d := range p.decisions {
		if d == "trust" || d == "deny" {
			decisions[name] = d
		}
	}
	s.sendCore(map[string]any{"type": "plugin_trust_decisions", "decisions": decisions})
	s.closeModal()
}

func (s *session) handlePluginTrustKey(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	n := s.pluginTrustRows()
	switch {
	case msg.String() == "up" || s.kbAny(msg, "nav_up", "nav_up_alt"):
		if n > 0 {
			s.modal.cursor = (s.modal.cursor - 1 + n) % n
		}
	case msg.String() == "down" || s.kbAny(msg, "nav_down", "nav_down_alt"):
		if n > 0 {
			s.modal.cursor = (s.modal.cursor + 1) % n
		}
	case msg.String() == "enter" || s.kb(msg, "select"):
		if s.modal.cursor == n-1 {
			s.applyPluginTrust()
			return s, nil
		}
		s.cyclePluginTrustDecision(s.modal.cursor)
	case msg.String() == " " || msg.String() == "space":
		s.cyclePluginTrustDecision(s.modal.cursor)
	case s.kbAny(msg, "close", "quit"):
		s.pluginTrust = nil
		s.closeModal()
	}
	return s, nil
}

func (s *session) renderPluginTrustModal() string {
	p := s.pluginTrust
	w := s.modalWidth(110)
	rowW := w - 4 // modal border(2) + padding(2)
	if rowW < 1 {
		rowW = 1
	}
	hiStyle := lipgloss.NewStyle().
		Background(lipgloss.Color(c.dim)).
		Foreground(lipgloss.Color(c.fg)).
		Width(rowW)
	truncStyle := lipgloss.NewStyle().MaxWidth(rowW)

	lines := []string{accentStyle.Render("◆ Plugin Trust")}
	if p != nil && len(p.entries) > 0 {
		lines = append(lines, dimStyle.Render(
			"  These project plugins are not loaded until you trust them. Trusted plugins can run hooks with your privileges."))
	}
	lines = append(lines, separatorStyle.Render(strings.Repeat("─", w-2)))

	if p == nil || len(p.entries) == 0 {
		lines = append(lines, dimStyle.Render("  (no untrusted project plugins)"))
		lines = append(lines, "")
		lines = append(lines, truncStyle.Render(dimStyle.Render("  esc close")))
		return modalBox(w, strings.Join(lines, "\n"))
	}

	lineBudget := s.height - 12
	if lineBudget < 1 {
		lineBudget = 1
	}
	if s.modal.cursor < 0 {
		s.modal.cursor = 0
	}
	if s.modal.cursor >= s.pluginTrustRows() {
		s.modal.cursor = s.pluginTrustRows() - 1
	}
	// Scroll window so long plugin lists never overflow the terminal.
	start := 0
	if s.modal.cursor >= lineBudget {
		start = s.modal.cursor - lineBudget + 1
	}
	end := start + lineBudget
	if end > len(p.entries) {
		end = len(p.entries)
	}
	s.modalItemRow = map[int]int{}
	for vi := start; vi < end; vi++ {
		e := p.entries[vi]
		marker, state := "  ", "no decision yet"
		switch p.decisions[e.Name] {
		case "trust":
			marker = accentStyle.Render("✓ ")
			state = "trusted"
		case "deny":
			marker = errStyle.Render("✗ ")
			state = "denied"
		}
		desc := state
		if e.Version != "" {
			desc += " · v" + e.Version
		}
		if e.Description != "" {
			desc += " · " + e.Description
		}
		row := fitListRow(marker, e.Name, desc, 2, rowW, "")
		row = truncStyle.Render(row)
		if vi == s.modal.cursor {
			row = hiStyle.Render(row)
		}
		s.modalItemRow[1+len(lines)] = vi
		lines = append(lines, row)
	}
	if len(p.entries) > end {
		lines = append(lines, dimStyle.Render(fmt.Sprintf("  (%d more · ↑↓ scroll)", len(p.entries)-end)))
	}
	lines = append(lines, "")
	// Apply & close row (the last selectable row).
	applyLabel := "✓ Apply & close"
	applyDesc := "record decisions and load trusted plugins now"
	if s.modal.cursor == s.pluginTrustRows()-1 {
		applyRow := fitListRow(accentStyle.Render("▸ "), applyLabel, applyDesc, 2, rowW, "")
		lines = append(lines, truncStyle.Render(hiStyle.Render(applyRow)))
	} else {
		lines = append(lines, truncStyle.Render(fitListRow("  ", applyLabel, applyDesc, 2, rowW, "")))
	}
	s.modalItemRow[1+len(lines)] = len(p.entries)
	lines = append(lines, "")
	lines = append(lines, truncStyle.Render(dimStyle.Render(
		"  ↑↓ navigate · enter/space cycle trust → deny → undecided · enter on apply saves · esc close")))
	return modalBox(w, strings.Join(lines, "\n"))
}
