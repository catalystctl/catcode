package main

import (
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
)

// ---------------------------------------------------------------------------
// Core event handling
// ---------------------------------------------------------------------------

func (s *session) handleCoreEvent(ev *coreEvent) tea.Cmd {
	switch ev.Type {
	case "ready":
		var models []modelInfo
		var m map[string]json.RawMessage
		if err := json.Unmarshal(ev.Raw, &m); err == nil {
			if raw, ok := m["models"]; ok {
				_ = json.Unmarshal(raw, &models)
			}
			if a, ok := m["authed"]; ok {
				var b bool
				_ = json.Unmarshal(a, &b)
				s.authed = b
			}
			if raw, ok := m["approval"]; ok {
				var mode string
				_ = json.Unmarshal(raw, &mode)
				s.approvalModeStr = mode
			}
			if raw, ok := m["bash_timeout_secs"]; ok {
				var n int
				_ = json.Unmarshal(raw, &n)
				if n > 0 {
					s.coreBashTimeout = n
				}
			}
			if raw, ok := m["max_turns"]; ok {
				var n int
				_ = json.Unmarshal(raw, &n)
				if n > 0 {
					s.coreMaxTurns = n
				}
			}
		}
		s.models = models
		s.modelIdx = 0
		// Apply persisted model selection if it matches a discovered model.
		if sel := s.settings.SelectedModel; sel != "" {
			for i, mm := range models {
				if mm.ID == sel || strings.Contains(mm.ID, sel) {
					s.modelIdx = i
					break
				}
			}
		} else {
			for i, mm := range models {
				if strings.Contains(mm.ID, "glm") {
					s.modelIdx = i
					break
				}
			}
		}
		// Keep the persisted reasoning effort valid for the selected model's
		// advertised thinking levels (clamps e.g. "medium" -> "high" on GLM).
		if s.clampReasoning() {
			_ = s.settings.save()
			if s.modelIdx >= 0 && s.modelIdx < len(s.models) {
				s.logInfo(fmt.Sprintf("reasoning: %s (for %s)", s.settings.ReasoningEffort, s.models[s.modelIdx].ID))
			}
		}
		s.logInfo(fmt.Sprintf("%d model(s) discovered", len(models)))
		// Re-authenticate with a persisted key, if any.
		if !s.authed && s.settings.APIKey != "" {
			s.sendCore(map[string]any{"type": "set_key", "api_key": s.settings.APIKey})
		} else if !s.authed {
			s.logWarn("set your key: /key sk-...  (or open settings)")
		}

	case "authed":
		s.authed = true
		s.logSuccess("authenticated")

	case "delta":
		if s.cur == nil || s.cur.kind != blkAssistant {
			s.push(blkAssistant)
		}
		if s.modelIdx >= 0 && s.modelIdx < len(s.models) {
			s.cur.model = s.models[s.modelIdx].ID
		}
		s.cur.text.WriteString(ev.get("text"))
		s.refresh()

	case "thinking":
		if s.cur == nil || s.cur.kind != blkThinking {
			s.push(blkThinking)
		}
		s.cur.text.WriteString(ev.get("text"))
		s.refresh()

	case "tool_call":
		name := ev.get("name")
		sub := strings.HasPrefix(name, "spawn:") // sub-agent internal call
		if sub {
			name = strings.TrimPrefix(name, "spawn:")
		}
		b := s.logTool(name, ev.get("args"), sub)
		b.id = ev.get("id")
		if !sub && name == "spawn" {
			s.layout() // a scout started: make room for the active-tasks panel
		}

	case "tool_result":
		out := ev.get("output")
		id := ev.get("id")
		// Match the result to its in-flight tool block by id. spawn nests its
		// sub-agent calls below the parent scout, so the parent's result doesn't
		// land on the last block — positional matching would misattribute it.
		var match *block
		for i := len(s.blocks) - 1; i >= 0; i-- {
			b := s.blocks[i]
			if b.kind == blkTool && b.dur == 0 && (id == "" || b.id == id) {
				match = b
				break
			}
		}
		if match != nil {
			match.output = out
			match.dur = time.Since(match.started)
			s.cur = nil
			wasScout := !match.sub && match.name == "spawn"
			s.invalidateAll()
			s.refresh()
			if wasScout {
				s.layout() // scout finished: release the active-tasks panel
			}
		} else {
			s.logToolResult(out)
		}

	case "done":
		if s.queuedNext {
			// A follow-up or steer turn begins right after this one; stay busy so the
			// footer keeps streaming and the input stays live.
			s.queuedNext = false
			s.cur = nil
			s.finalizeInFlight("")
			s.layout()
			s.logInfo("continuing queued turn…")
		} else {
			s.busy = false
			s.turnCount++
			s.cur = nil
			s.finalizeInFlight("[no result]")
			s.layout()
			s.logSuccess(fmt.Sprintf("turn %d complete", s.turnCount))
			s.input.Focus()
		}

	case "aborted":
		if s.queuedNext {
			// A steer interrupted this turn; the steered turn runs next. Stay busy
			// (a `done` is still coming for the interrupted turn) and drop the
			// partial output without an "aborted" label.
			s.cur = nil
			s.finalizeInFlight("")
			s.layout()
		} else {
			s.busy = false
			s.cur = nil
			s.finalizeInFlight("[aborted]")
			s.layout()
			s.logWarn("aborted")
			s.input.Focus()
		}

	case "reset":
		s.blocks = nil
		s.cur = nil
		s.contextTokens = 0
		s.follow = true
		s.invalidateAll()
		s.logInfo("conversation reset")

	case "history":
		var m map[string]json.RawMessage
		if json.Unmarshal(ev.Raw, &m) == nil {
			if raw, ok := m["messages"]; ok {
				var msgs []map[string]json.RawMessage
				if json.Unmarshal(raw, &msgs) == nil {
					s.rebuildBlocksFromHistory(msgs)
					s.follow = true
					s.invalidateAll()
					s.refresh()
				}
			}
		}
	case "compacted":
		s.logInfo(fmt.Sprintf("context compacted: %s → %s tokens", ev.get("before_tokens"), ev.get("after_tokens")))

	case "approval_changed":
		s.approvalModeStr = ev.get("mode")
		s.logInfo(fmt.Sprintf("approval mode: %s", ev.get("mode")))

	case "config_changed":
		s.logInfo(fmt.Sprintf("config: %s = %s", ev.get("key"), ev.get("value")))

	case "http_retry":
		s.logInfo(fmt.Sprintf("retry #%s %s (%s ms)", ev.get("attempt"), ev.get("status"), ev.get("backoff_ms")))

	case "metrics":
		s.lastMetrics = ev.Raw
		// tokens_in/out are cumulative session totals from the core.
		if ti, err := strconv.ParseUint(ev.get("tokens_in"), 10, 64); err == nil {
			if to, err := strconv.ParseUint(ev.get("tokens_out"), 10, 64); err == nil {
				s.contextTokens = ti + to
			}
		}

	case "approval_request":
		s.pendingApproval = &approvalPrompt{
			requestID: ev.get("request_id"),
			tool:      ev.get("tool"),
			args:      ev.get("args"),
		}
		s.logApprove(ev.get("tool"), ev.get("args"))
		s.input.Focus()
		s.layout() // banner claims a line, shrink viewport

	case "info":
		s.logInfo(ev.get("message"))

	case "steer":
		// Core acknowledged a steer: the running turn was interrupted and the
		// steered turn is starting. The user message was already logged on send,
		// so just mark the redirect.
		s.logInfo("steering…")

	case "sessions":
		var entries []sessionEntry
		var m map[string]json.RawMessage
		if err := json.Unmarshal(ev.Raw, &m); err == nil {
			if raw, ok := m["sessions"]; ok {
				_ = json.Unmarshal(raw, &entries)
			}
		}
		if len(entries) == 0 {
			s.logInfo("no saved sessions found")
		} else {
			s.sessionList = entries
			s.openSessionsPicker()
		}

	case "stats":
		ti := ev.get("tokens_in")
		to := ev.get("tokens_out")
		tt := ev.get("tokens_total")
		turns := ev.get("turns")
		msgs := ev.get("messages")
		s.logInfo(fmt.Sprintf("stats: %s in / %s out (%s total) · %s turns · %s msgs", ti, to, tt, turns, msgs))

	case "error":
		s.logError(ev.get("message"))
	case "plugin_installed":
		s.logSuccess(fmt.Sprintf("plugin installed: %s v%s — %s", ev.get("name"), ev.get("version"), ev.get("description")))
	case "plugin_removed":
		s.logInfo(fmt.Sprintf("plugin removed: %s", ev.get("name")))
	case "plugin_enabled":
		s.logInfo(fmt.Sprintf("plugin enabled: %s", ev.get("name")))
	case "plugin_disabled":
		s.logInfo(fmt.Sprintf("plugin disabled: %s", ev.get("name")))
	case "plugin_error":
		s.logError(fmt.Sprintf("plugin error (%s): %s", ev.get("name"), ev.get("message")))
	case "plugins_list":
		var m map[string]json.RawMessage
		if err := json.Unmarshal(ev.Raw, &m); err == nil {
			if raw, ok := m["plugins"]; ok {
				var plugins []json.RawMessage
				if json.Unmarshal(raw, &plugins) == nil {
					s.openPluginPicker(plugins)
				}
			}
		}
	}
	return waitForEvent(s.coreEvents)
}

// sendSteer dispatches a steer command to the core: interrupt the running
// turn (if any) and redirect it with prompt. Marks a chained turn so the TUI
// keeps streaming across the interrupt. Used by both Ctrl+Enter and /steer.
func (s *session) sendSteer(prompt string) tea.Cmd {
	if !s.authed {
		s.logError("not authenticated — run /key sk-... first")
		return nil
	}
	if len(s.models) == 0 {
		s.logError("no models loaded yet")
		return nil
	}
	model := s.models[s.modelIdx].ID
	s.follow = true
	s.logUser(prompt + "  ↳ steer")
	s.pushHistory(prompt)
	s.queuedNext = true
	s.sendCore(map[string]any{
		"type":             "steer",
		"prompt":           prompt,
		"model":            model,
		"reasoning_effort": s.settings.ReasoningEffort,
	})
	return nil
}

// steerFromInput sends the current input as a steer (Ctrl+Enter).
func (s *session) steerFromInput() tea.Cmd {
	text := strings.TrimSpace(s.input.Value())
	if text == "" {
		return nil
	}
	s.input.Reset()
	return s.sendSteer(text)
}

// queueFollowUp sends prompt as a follow-up: the core buffers it (one-deep)
// and runs it after the current turn. Marks a chained turn so the TUI stays
// busy across the hand-off instead of flashing "ready".
func (s *session) queueFollowUp(text string) tea.Cmd {
	if !s.authed {
		s.logError("not authenticated — run /key sk-... first")
		return nil
	}
	if len(s.models) == 0 {
		s.logError("no models loaded yet")
		return nil
	}
	model := s.models[s.modelIdx].ID
	s.follow = true
	s.logUser(text + "  ↳ queued")
	s.pushHistory(text)
	s.queuedNext = true
	s.sendCore(map[string]any{
		"type":             "send",
		"prompt":           text,
		"model":            model,
		"reasoning_effort": s.settings.ReasoningEffort,
	})
	return nil
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

func (s *session) handleKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	// global: Ctrl+C quits (unless a modal is open, where esc closes)
	if msg.Type == tea.KeyCtrlC && s.modal.kind == modalNone {
		if s.coreCmd != nil && s.coreCmd.Process != nil {
			_ = s.coreCmd.Process.Kill()
		}
		return s, tea.Quit
	}
	// modal intercept: when a modal is active it owns all keys.
	if s.modal.kind != modalNone {
		return s.handleModalKey(msg)
	}
	// transcript scrolling works in every state (idle/busy/approval) so the
	// user can read history while a turn runs or a decision is pending.
	if s.handleScrollKey(msg) {
		return s, nil
	}
	// global: ctrl+t toggles reasoning-block collapse/expand
	if msg.String() == "ctrl+t" {
		s.thinkExpanded = !s.thinkExpanded
		s.settings.ThinkExpanded = s.thinkExpanded
		_ = s.settings.save()
		for _, b := range s.blocks {
			if b.kind == blkThinking {
				b.collapsed = !s.thinkExpanded
			}
		}
		s.invalidateAll()
		s.refresh()
		return s, nil
	}
	// global: ctrl+o toggles full output for the most recent tool block.
	// Tool output is truncated to the first 3 lines by default; this expands
	// (or collapses) the last tool call so the user can inspect full output.
	if msg.String() == "ctrl+o" {
		if b := s.lastToolOutputBlock(); b != nil {
			b.expanded = !b.expanded
			s.invalidateAll()
			s.refresh()
		}
		return s, nil
	}
	// global: ctrl+p / ctrl+k opens the command palette.
	if msg.String() == "ctrl+p" || msg.String() == "ctrl+k" {
		s.openCommandPalette()
		return s, nil
	}
	// global: ctrl+r opens the reasoning-effort picker for the active model.
	if msg.String() == "ctrl+r" {
		s.openReasoningPicker()
		return s, nil
	}
	// "/" opens the palette when the input is empty and idle.
	if msg.String() == "/" && s.input.Value() == "" && !s.busy {
		s.openCommandPalette()
		return s, nil
	}
	if s.pendingApproval != nil {
		switch msg.String() {
		case "y", "Y":
			s.sendCore(map[string]any{"type": "approve", "request_id": s.pendingApproval.requestID, "decision": "yes"})
			s.pendingApproval = nil
		case "a", "A":
			s.sendCore(map[string]any{"type": "approve", "request_id": s.pendingApproval.requestID, "decision": "always"})
			s.pendingApproval = nil
		case "n", "N", "esc":
			s.sendCore(map[string]any{"type": "approve", "request_id": s.pendingApproval.requestID, "decision": "no"})
			s.logError("denied")
			s.pendingApproval = nil
		}
		s.layout() // banner released, grow viewport back
		return s, nil
	}
	if s.busy {
		// While a turn runs the input stays live: type a follow-up (Enter),
		// steer the model (Ctrl+Enter), run a slash command, or abort (Esc).
		// Scrolling + ctrl+t/o/p above still work; a typed "/" composes a slash
		// command (the lone-"/" palette opener stays idle-only).
		switch msg.String() {
		case "esc":
			// Esc aborts the turn and drops any queued follow-up/steer.
			s.queuedNext = false
			s.sendCore(map[string]any{"type": "abort"})
			s.logWarn("aborting…")
			return s, nil
		case "ctrl+enter":
			return s, s.steerFromInput()
		}
		if msg.Type == tea.KeyEnter {
			text := strings.TrimSpace(s.input.Value())
			if text == "" {
				return s, nil
			}
			s.input.Reset()
			if strings.HasPrefix(text, "/") {
				return s, s.handleUserLine(text)
			}
			return s, s.queueFollowUp(text)
		}
		var cmd tea.Cmd
		s.input, cmd = s.input.Update(msg)
		return s, cmd
	}

	// welcome-screen navigation: when the conversation is empty, ↑/↓ move the
	// example cursor; enter drops the selected example into the (editable)
	// input. Only arrow keys are used so typing letters/digits is unaffected.
	if len(s.blocks) == 0 && s.pendingApproval == nil {
		switch msg.String() {
		case "up":
			s.welcomeIdx = (s.welcomeIdx - 1 + len(welcomeExamples)) % len(welcomeExamples)
			return s, nil
		case "down":
			s.welcomeIdx = (s.welcomeIdx + 1) % len(welcomeExamples)
			return s, nil
		}
		if msg.Type == tea.KeyEnter && strings.TrimSpace(s.input.Value()) == "" {
			s.input.SetValue(welcomeExamples[s.welcomeIdx])
			s.input.Focus()
			return s, nil
		}
	}

	// history recall: up/down when the input is focused and not empty-positioned
	if msg.String() == "up" && len(s.history) > 0 {
		val := s.recallHistory(-1)
		s.input.SetValue(val)
		return s, nil
	}
	if msg.String() == "down" && len(s.history) > 0 {
		val := s.recallHistory(+1)
		s.input.SetValue(val)
		return s, nil
	}

	if msg.Type == tea.KeyEnter {
		text := strings.TrimSpace(s.input.Value())
		if text == "" {
			return s, nil
		}
		s.input.Reset()
		s.histIdx = len(s.history)
		return s, s.handleUserLine(text)
	}

	s.input, _ = s.input.Update(msg)
	return s, nil
}

// handleScrollKey moves the transcript viewport and manages follow mode.
// Returns true when it consumed the key. Scroll-up motions pause follow (so the
// view isn't yanked to the bottom on the next token); scroll-down re-pins
// follow once the bottom is reached.
func (s *session) handleScrollKey(msg tea.KeyMsg) bool {
	switch msg.String() {
	case "pgup":
		s.follow = false
		s.viewport.PageUp()
		return true
	case "pgdown":
		s.viewport.PageDown()
		if s.viewport.AtBottom() {
			s.follow = true
		}
		return true
	case "ctrl+up":
		s.follow = false
		s.viewport.LineUp(1)
		return true
	case "ctrl+down":
		s.viewport.LineDown(1)
		if s.viewport.AtBottom() {
			s.follow = true
		}
		return true
	case "ctrl+home":
		s.follow = false
		s.viewport.GotoTop()
		return true
	case "ctrl+end":
		s.follow = true
		s.viewport.GotoBottom()
		return true
	}
	return false
}

// ---------------------------------------------------------------------------
// User line / slash commands
// ---------------------------------------------------------------------------

func (s *session) handleUserLine(text string) tea.Cmd {
	if strings.HasPrefix(text, "/") {
		parts := strings.Fields(text)
		switch parts[0] {
		case "/key":
			if len(parts) < 2 {
				s.logError("usage: /key sk-...")
				return nil
			}
			s.settings.APIKey = parts[1]
			_ = s.settings.save()
			s.sendCore(map[string]any{"type": "set_key", "api_key": parts[1]})
			s.logInfo("sending key…")
			return nil
		case "/model":
			if len(parts) < 2 {
				s.openModelPicker()
				return nil
			}
			idx := -1
			if n, _ := fmt.Sscanf(parts[1], "%d", &idx); n == 1 && idx >= 0 && idx < len(s.models) {
			} else {
				idx = -1
				for i, mm := range s.models {
					if strings.Contains(mm.ID, parts[1]) {
						idx = i
						break
					}
				}
			}
			if idx < 0 {
				s.logError("no model matches '" + parts[1] + "'")
				return nil
			}
			s.modelIdx = idx
			s.settings.SelectedModel = s.models[idx].ID
			_ = s.settings.save()
			if s.clampReasoning() {
				_ = s.settings.save()
				s.logInfo(fmt.Sprintf("reasoning: %s (for %s)", s.settings.ReasoningEffort, s.models[idx].ID))
			}
			s.logInfo(fmt.Sprintf("model: %s", s.models[idx].ID))
			return nil
		case "/reset":
			s.sendCore(map[string]any{"type": "reset"})
			s.blocks = nil
			s.cur = nil
			s.contextTokens = 0
			s.follow = true
			s.invalidateAll()
			s.viewport.SetContent("")
			return nil
		case "/abort":
			s.queuedNext = false
			s.sendCore(map[string]any{"type": "abort"})
			return nil
		case "/steer":
			// Steer via command: works on every terminal (Ctrl+Enter is only detected
			// on terminals that send a distinct CSI for it).
			if len(parts) < 2 {
				s.logError("usage: /steer <message>")
				return nil
			}
			return s.sendSteer(strings.Join(parts[1:], " "))
		case "/approval":
			if len(parts) < 2 {
				s.logError("usage: /approval never|destructive|always")
				return nil
			}
			s.settings.Approval = parts[1]
			_ = s.settings.save()
			s.sendCore(map[string]any{"type": "set_approval", "mode": parts[1]})
			return nil
		case "/help", "/?":
			s.openHelp()
			return nil
		case "/settings":
			s.openSettings()
			return nil
		case "/theme":
			s.openThemePicker()
			return nil
		case "/copy":
			return s.copyLastAssistant()
		case "/attach":
			// /attach <path> [prompt] — send the current input (or the given prompt) with an image.
			if len(parts) < 2 {
				s.logError("usage: /attach <image-path> [optional prompt]")
				return nil
			}
			imgPath := parts[1]
			// Remaining parts joined become the prompt; fall back to the current input value.
			promptText := ""
			if len(parts) > 2 {
				promptText = strings.Join(parts[2:], " ")
			} else {
				promptText = s.input.Value()
			}
			if strings.TrimSpace(promptText) == "" {
				promptText = "Describe this image."
			}
			if !s.authed {
				s.logError("not authenticated — run /key sk-... first")
				return nil
			}
			if len(s.models) == 0 {
				s.logError("no models loaded yet")
				return nil
			}
			model := s.models[s.modelIdx].ID
			s.follow = true // jump to bottom so the user sees their turn
			s.logUser(promptText + " [image: " + imgPath + "]")
			s.sendCore(map[string]any{
				"type":             "send",
				"prompt":           promptText,
				"model":            model,
				"reasoning_effort": s.settings.ReasoningEffort,
				"images":           []string{imgPath},
			})
			s.busy = true
			s.input.Reset()
			return nil
		case "/clear":
			s.sendCore(map[string]any{"type": "clear"})
			s.blocks = nil
			s.cur = nil
			s.contextTokens = 0
			s.follow = true
			s.invalidateAll()
			s.viewport.SetContent("")
			s.logInfo("in-memory conversation cleared (session file kept)")
			return nil
		case "/undo":
			s.sendCore(map[string]any{"type": "undo"})
			s.blocks = nil
			s.cur = nil
			s.contextTokens = 0
			s.invalidateAll()
			s.viewport.SetContent("")
			s.logInfo("dropped last turn")
			return nil
		case "/compact":
			s.sendCore(map[string]any{"type": "compact"})
			s.logInfo("forcing context compaction…")
			return nil
		case "/sessions":
			s.sendCore(map[string]any{"type": "list_sessions"})
			return nil
		case "/new":
			s.sendCore(map[string]any{"type": "new_session", "path": newSessionFilename()})
			s.blocks = nil
			s.cur = nil
			s.contextTokens = 0
			s.follow = true
			s.invalidateAll()
			s.viewport.SetContent("")
			s.logInfo("starting a new session…")
			return nil
		case "/stats":
			s.sendCore(map[string]any{"type": "stats"})
			return nil
		case "/plugin-install":
			if len(parts) < 2 {
				s.logError("usage: /plugin-install <path-to-plugin-dir>")
				return nil
			}
			s.sendCore(map[string]any{"type": "install_plugin", "path": parts[1]})
			s.logInfo(fmt.Sprintf("installing plugin from %s…", parts[1]))
			return nil
		case "/plugin-list":
			s.sendCore(map[string]any{"type": "list_plugins"})
			return nil
		case "/plugin-enable":
			if len(parts) < 2 {
				s.logError("usage: /plugin-enable <name>")
				return nil
			}
			s.sendCore(map[string]any{"type": "enable_plugin", "name": parts[1]})
			return nil
		case "/plugin-disable":
			if len(parts) < 2 {
				s.logError("usage: /plugin-disable <name>")
				return nil
			}
			s.sendCore(map[string]any{"type": "disable_plugin", "name": parts[1]})
			return nil
		case "/plugin-remove":
			if len(parts) < 2 {
				s.logError("usage: /plugin-remove <name>")
				return nil
			}
			s.sendCore(map[string]any{"type": "remove_plugin", "name": parts[1]})
			return nil
		default:
			s.logError("unknown command: " + parts[0])
			return nil
		}
	}

	if !s.authed {
		s.logError("not authenticated — run /key sk-... first")
		return nil
	}
	if len(s.models) == 0 {
		s.logError("no models loaded yet")
		return nil
	}
	model := s.models[s.modelIdx].ID
	s.follow = true // jump to bottom so the user sees their turn + the response
	s.logUser(text)
	s.pushHistory(text)
	s.sendCore(map[string]any{
		"type":             "send",
		"prompt":           text,
		"model":            model,
		"reasoning_effort": s.settings.ReasoningEffort,
	})
	s.busy = true
	return nil
}
