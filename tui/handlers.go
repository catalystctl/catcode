package main

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"runtime"
	"strconv"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
)

// ---------------------------------------------------------------------------
// Core event handling
// ---------------------------------------------------------------------------

// accumulateSaved adds a context-management event's reclaimed tokens
// (before_tokens − after_tokens) to the session's cumulative counter. Shared by
// the "digested" and "compacted" events; both carry the same before/after
// fields, and because each event reports only its own delta they add up without
// double-counting across the soft-digest and compaction tiers.
func (s *session) accumulateSaved(ev *coreEvent) {
	before, err1 := strconv.ParseUint(ev.get("before_tokens"), 10, 64)
	after, err2 := strconv.ParseUint(ev.get("after_tokens"), 10, 64)
	if err1 != nil || err2 != nil || before > after {
		return
	}
	s.tokensSaved += before - after
}

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
			// Provider fields (openai/anthropic endpoints).
			if raw, ok := m["provider"]; ok {
				_ = json.Unmarshal(raw, &s.activeProvider)
			}
			if raw, ok := m["providerKind"]; ok {
				_ = json.Unmarshal(raw, &s.providerKind)
			}
			if raw, ok := m["providers"]; ok {
				_ = json.Unmarshal(raw, &s.providers)
			}
			if raw, ok := m["providerPresets"]; ok {
				_ = json.Unmarshal(raw, &s.providerPresets)
			}
			s.providerHasKey = s.authed // ready's authed reflects the active provider's key
		}
		s.applyModels(models)
		s.logInfo(fmt.Sprintf("%d model(s) discovered", len(models)))
		// Sync a persisted provider selection that differs from the core's
		// startup choice (e.g. switched in a previous session). The core emits
		// provider_changed + models, which re-resolves the key below.
		if s.settings.ActiveProvider != "" && s.settings.ActiveProvider != s.activeProvider &&
			s.containsProvider(s.settings.ActiveProvider) {
			s.sendCore(map[string]any{"type": "set_provider", "name": s.settings.ActiveProvider})
			// Re-arm the core-event pump: returning nil here schedules no further
			// waitForEvent, so no core event would ever be processed again — the
			// spinner keeps ticking but nothing streams (TUI deadlock). The
			// set_provider round-trip is async; provider_changed will arrive and
			// be handled normally on the next pump tick.
			return waitForEvent(s.coreEvents)
		}
		s.reauthActiveProvider()

	case "provider_changed":
		var m map[string]json.RawMessage
		if err := json.Unmarshal(ev.Raw, &m); err == nil {
			_ = json.Unmarshal([]byte(get(m, "provider")), &s.activeProvider)
			_ = json.Unmarshal([]byte(get(m, "kind")), &s.providerKind)
			if hk := get(m, "has_key"); hk != "" {
				var b bool
				_ = json.Unmarshal([]byte(hk), &b)
				s.providerHasKey = b
			}
		}
		// Persist the selection so the next session restores it.
		if s.activeProvider != "" && s.settings.ActiveProvider != s.activeProvider {
			s.settings.ActiveProvider = s.activeProvider
			_ = s.settings.save()
		}
		s.reauthActiveProvider()

	case "authed":
		s.authed = true
		s.providerHasKey = true
		s.logSuccess("authenticated")

	case "provider_presets":
		// The core advertises the first-party presets (and refreshes them after
		// add_provider so Configured/HasKey flip). Keep the picker open if it's
		// up so the list updates live.
		var presets []providerPreset
		if raw := ev.get("presets"); raw != "" {
			_ = json.Unmarshal([]byte(raw), &presets)
		} else {
			var m map[string]json.RawMessage
			if err := json.Unmarshal(ev.Raw, &m); err == nil {
				if raw, ok := m["presets"]; ok {
					_ = json.Unmarshal(raw, &presets)
				}
			}
		}
		s.providerPresets = presets
		if s.modal.kind == modalProviders {
			s.refresh()
		}

	case "models":
		// The core emits this after a provider switch (and on demand). Re-apply the
		// model list + persisted selection exactly like the ready path.
		var models []modelInfo
		if raw := ev.get("models"); raw != "" {
			_ = json.Unmarshal([]byte(raw), &models)
		} else {
			var m map[string]json.RawMessage
			if err := json.Unmarshal(ev.Raw, &m); err == nil {
				if raw, ok := m["models"]; ok {
					_ = json.Unmarshal(raw, &models)
				}
			}
		}
		s.applyModels(models)
		s.logInfo(fmt.Sprintf("%d model(s) discovered", len(models)))

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
		sub := strings.HasPrefix(name, "spawn:") || strings.HasPrefix(name, "subagent:") // sub-agent internal call
		if sub {
			name = strings.TrimPrefix(strings.TrimPrefix(name, "spawn:"), "subagent:")
		}
		b := s.logTool(name, ev.get("args"), sub)
		b.id = ev.get("id")
		if name == "todo_write" {
			// Capture the latest todo list so the pinned panel always reflects
			// current state (the agent rewrites the full list each call).
			s.captureTodos(b.args)
			s.layout() // todo panel may have appeared/grown
		}
		if !sub && (name == "spawn" || name == "subagent") {
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
			match.diff = ev.get("diff")
			match.ok = ev.get("ok") == "true"
			match.hasOk = true
			match.dur = time.Since(match.started)
			s.cur = nil
			wasScout := !match.sub && (match.name == "spawn" || match.name == "subagent")
			s.invalidateAll()
			s.refresh()
			if wasScout {
				s.layout() // scout finished: release the active-tasks panel
			}
		} else {
			s.logToolResult(out)
		}

	case "done":
		s.subProgress = nil
		// When every task is complete, dismiss the pinned tasks panel — a
		// finished plan shouldn't linger as a permanent fixture. Done before
		// the layout() calls below so the cleared state renders immediately.
		// A later todo_write (new work) re-shows it.
		if s.allTodosComplete() {
			s.todos = nil
		}
		if s.queuedNext {
			// A follow-up or steer turn begins right after this one; stay busy so the
			// footer keeps streaming and the input stays live.
			s.queuedNext = false
			s.queued = nil
			s.cur = nil
			s.finalizeInFlight("")
			s.layout()
			s.logInfo("continuing queued turn…")
		} else {
			s.busy = false
			s.turnCount++
			s.coreRestarts = 0 // P1-17: a completed turn resets the crash-restart budget
			s.cur = nil
			s.finalizeInFlight("[no result]")
			s.layout()
			s.logSuccess(fmt.Sprintf("turn %d complete", s.turnCount))
			s.input.Focus()
		}
		// Refresh the discoverable-skills list so a skill created mid-turn
		// (e.g. by /reflect or /index) shows up in the /skill:<name> autocomplete.
		s.sendCore(map[string]any{"type": "list_skills"})

	case "aborted":
		s.subProgress = nil
		if s.queuedNext {
			// A steer interrupted this turn; the steered turn runs next. Clear the
			// queued flag here so the steered turn's terminal `done` falls through
			// to the else branch and clears `busy` — otherwise `busy` stays true
			// forever and the TUI wedges (only Ctrl+C recovers) (P0-5).
			s.queuedNext = false
			s.queued = nil
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
		s.lastCachePct = 0
		s.tokensSaved = 0
		s.summaryChars = 0
		s.subProgress = nil
		s.todos = nil
		s.queued = nil
		s.queuedNext = false
		s.follow = true
		s.invalidateAll()
		s.layout()
		s.logInfo("conversation reset")

	case "history":
		var m map[string]json.RawMessage
		if json.Unmarshal(ev.Raw, &m) == nil {
			if raw, ok := m["messages"]; ok {
				var msgs []map[string]json.RawMessage
				if json.Unmarshal(raw, &msgs) == nil {
					s.rebuildBlocksFromHistory(msgs)
					// Show the loaded session's used context immediately instead
					// of waiting for the first turn's metrics event.
					if ti, err := strconv.ParseUint(ev.get("tokens_in"), 10, 64); err == nil {
						s.contextTokens = ti
					}
					s.follow = true
					s.invalidateAll()
					s.refresh()
				}
			}
		}
	case "compacted":
		if ev.get("scope") == "subagent" {
			break // subagent-internal compaction; don't clutter the main transcript
		}
		// The footer's context budget is driven by metrics events, which only
		// fire at turn end. Reflect the post-compaction size now so the bar
		// doesn't keep showing a stale, over-full count after a compact/digest.
		if at, err := strconv.ParseUint(ev.get("after_tokens"), 10, 64); err == nil {
			s.contextTokens = at
		}
		s.logInfo(fmt.Sprintf("context compacted: %s → %s tokens", ev.get("before_tokens"), ev.get("after_tokens")))

	case "digested":
		if at, err := strconv.ParseUint(ev.get("after_tokens"), 10, 64); err == nil {
			s.contextTokens = at
		}
		s.logInfo(fmt.Sprintf("context digested: %s result(s), %s → %s tokens", ev.get("results"), ev.get("before_tokens"), ev.get("after_tokens")))

	case "reflecting":
		// The auto-reflect seam fired: instead of completing on `finish`, the core
		// injected a reflection continuation so durable facts (memory) and recurring
		// patterns (skills) get persisted without relying on the model remembering
		// to. Surfaced so the post-finish model activity isn't a mystery.
		if n := ev.get("recurrence"); n != "" && n != "0" {
			s.logInfo(fmt.Sprintf("auto-reflect: reflecting on this turn (%s recurring patterns)…", n))
		} else {
			s.logInfo("auto-reflect: reflecting on this turn…")
		}

	case "approval_changed":
		s.approvalModeStr = ev.get("mode")
		s.logInfo(fmt.Sprintf("approval mode: %s", ev.get("mode")))

	case "config_changed":
		s.logInfo(fmt.Sprintf("config: %s = %s", ev.get("key"), ev.get("value")))

	case "http_retry":
		s.logInfo(fmt.Sprintf("retry #%s %s (%s ms)", ev.get("attempt"), ev.get("status"), ev.get("backoff_ms")))

	case "metrics":
		s.lastMetrics = ev.Raw
		// tokens_in is the live context size (prompt + output). During a turn the
		// core emits periodic estimates so the footer moves; at turn end the real
		// usage overwrites them. prompt_tokens (when present) is the prompt-only
		// count, used for the cached-fraction denominator.
		if ti, err := strconv.ParseUint(ev.get("tokens_in"), 10, 64); err == nil {
			s.contextTokens = ti
		}
		// The mid-stream metrics event omits cached_tokens; it only lands at turn
		// end. Capture the per-turn cache hit rate here (cached / prompt_tokens,
		// falling back to tokens_in) so renderMetrics can keep showing a cache %
		// while the *next* turn is in flight — carried and marked "~".
		if cached := ev.get("cached_tokens"); cached != "" && cached != "null" && cached != "0" {
			tin := ev.get("prompt_tokens")
			if tin == "" || tin == "null" || tin == "0" {
				tin = ev.get("tokens_in")
			}
			if tin != "" && tin != "null" && tin != "0" {
				if cN, err := strconv.ParseUint(cached, 10, 64); err == nil && cN > 0 {
					if tN, err := strconv.ParseUint(tin, 10, 64); err == nil && tN > 0 {
						s.lastCachePct = int(cN * 100 / tN)
					}
				}
			}
		}

	case "umans_conc":
		// Live account-wide concurrency from the Umans gateway's /v1/usage poll
		// (core background task). used=nil => not Umans / fetch failed → hide;
		// used set + limit=nil => unlimited → render ∞. `provider` is the Umans
		// provider the poll tracks; renderUmansConc only shows the field when the
		// selected model routes to it (a Gemini/OpenAI model selected → hidden).
		s.umansConcUsed = nullableInt64(ev.get("used"))
		s.umansConcLimit = nullableInt64(ev.get("limit"))
		s.umansConcProvider = ev.get("provider")

	case "approval_request":
		s.pendingApproval = &approvalPrompt{
			requestID: ev.get("request_id"),
			tool:      ev.get("tool"),
			args:      ev.get("args"),
			diff:      ev.get("diff"),
		}
		s.logApproveDiff(ev.get("tool"), ev.get("args"), ev.get("diff"))
		s.input.Focus()
	case "intercom_message":
		// A subagent is prompting the orchestrator for a decision (or a progress
		// update). need_decision blocks until we reply; progress_update is a log line.
		reason := ev.get("reason")
		if reason == "progress_update" {
			s.logInfo(fmt.Sprintf("⟵ %s (progress): %s", ev.get("from"), ev.get("message")))
		} else {
			s.pendingIntercom = &intercomPrompt{
				requestID: ev.get("id"),
				from:      ev.get("from"),
				reason:    reason,
				message:   ev.get("message"),
			}
			s.logWarn(fmt.Sprintf("⟵ subagent %s asks: %s", ev.get("from"), ev.get("message")))
			s.input.SetValue("")
			s.input.Focus()
			s.layout()
		}

	case "subagent_progress":
		runID := ev.get("run_id")
		if ev.get("phase") == "done" {
			if runID != "" {
				s.removeSubProgress(runID)
			}
			s.layout()
			break
		}
		entry := s.findSubProgress(runID)
		if entry == nil {
			entry = &subProgressEntry{runID: runID, agent: ev.get("agent"), started: time.Now()}
			s.subProgress = append(s.subProgress, entry)
			s.layout()
		}
		if tc := ev.get("tool_count"); tc != "" {
			if n, err := strconv.Atoi(tc); err == nil {
				entry.toolCount = n
			}
		}
		if ti := ev.get("tokens_in"); ti != "" {
			if n, err := strconv.ParseUint(ti, 10, 64); err == nil {
				entry.tokensIn = n
			}
		}
		if to := ev.get("tokens_out"); to != "" {
			if n, err := strconv.ParseUint(to, 10, 64); err == nil {
				entry.tokensOut = n
			}
		}
		switch ev.get("phase") {
		case "tool":
			entry.curTool = ev.get("tool")
			entry.toolStart = time.Now()
			entry.toolRunning = true
		case "tool_end":
			entry.toolRunning = false
			entry.curTool = ev.get("tool")
		case "streaming":
			entry.toolRunning = false
		}
		s.refresh()

	case "info":
		// Informational notices from the core (first-run staging, subagent
		// lifecycle, plugin handoffs, etc.). Surface them in the transcript.
		if msg := ev.get("message"); msg != "" {
			s.logInfo(msg)
		}

	case "oauth_prompt":
		// The core needs the user to complete an interactive OAuth login (visit a
		// URL and, for the device flow, enter a code). Surface it prominently in the
		// transcript and try to open the URL via the OS.
		url := ev.get("url")
		code := ev.get("code")
		message := ev.get("message")
		if message == "" {
			message = "complete the OAuth login in your browser"
		}
		// Copy the URL to the LOCAL clipboard via OSC 52 — works over SSH in
		// most modern terminals (iTerm2/kitty/WezTerm/Windows Terminal/
		// gnome-terminal/alacritty): the escape sequence passes through to the
		// local terminal, which writes its clipboard, so the user can just paste.
		// Best-effort: terminals that lack OSC 52 (e.g. macOS Terminal.app) ignore
		// it and the user copies from the hard-wrapped URL shown below instead.
		copyToClipboardOSC52(url)
		var b strings.Builder
		b.WriteString(message)
		if url != "" {
			b.WriteString("\n  url (copied to your clipboard — just paste into a browser; if paste is empty your terminal lacks OSC 52 — copy from below):")
			ww := s.width - 6
			if ww < 20 {
				ww = 20
			} else if ww > 200 {
				ww = 200
			}
			for _, ln := range wrapRunes([]rune(url), ww) {
				b.WriteString("\n    ")
				b.WriteString(string(ln))
			}
		}
		if code != "" {
			b.WriteString("\n  code: ")
			b.WriteString(code)
		}
		s.logInfo(b.String())
		if url != "" {
			openURL(url)
		}

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
		// tokens_in is the CURRENT real context (matches the footer); tokens_out
		// is cumulative output. total_in is the cumulative prompt (billing) and
		// drives the cache ratio.
		ti := ev.get("tokens_in")
		to := ev.get("tokens_out")
		totalIn := ev.get("total_in")
		cached := ev.get("cached_tokens")
		ratio := ev.get("cache_hit_ratio")
		turns := ev.get("turns")
		msgs := ev.get("messages")
		sessionFile := ev.get("session_file")
		s.logInfo(fmt.Sprintf("stats: %s in / %s out · %s turns · %s msgs", ti, to, turns, msgs))
		if totalIn != "" && totalIn != "0" {
			s.logInfo(fmt.Sprintf("totals: %s prompt in / %s out (cumulative)", totalIn, to))
		}
		if sessionFile != "" {
			s.logInfo(fmt.Sprintf("session: %s", sessionFile))
		}
		if cached != "" && cached != "0" {
			if ratio != "" {
				if r, err := strconv.ParseFloat(ratio, 64); err == nil {
					s.logSuccess(fmt.Sprintf("cache: %s cached · %.0f%% hit", cached, r*100))
				} else {
					s.logSuccess(fmt.Sprintf("cache: %s cached", cached))
				}
			} else {
				s.logSuccess(fmt.Sprintf("cache: %s cached", cached))
			}
		}

	case "memory_saved":
		if msg := ev.get("message"); msg != "" {
			s.logSuccess(msg)
		} else {
			s.logInfo("memory saved")
		}
	case "memory_list":
		var m map[string]json.RawMessage
		var entries []memoryEntry
		if err := json.Unmarshal(ev.Raw, &m); err == nil {
			if raw, ok := m["entries"]; ok {
				_ = json.Unmarshal(raw, &entries)
			}
		}
		if len(entries) == 0 {
			s.logInfo("no memories saved")
			break
		}
		var rows []string
		rows = append(rows, accentStyle.Render("◆ Memories"))
		for _, e := range entries {
			text := truncateRunes(e.Text, 80)
			id := e.ID
			if id == "" {
				id = "?"
			}
			tags := ""
			if len(e.Tags) > 0 {
				tags = "  " + dimStyle.Render("["+strings.Join(e.Tags, ",")+"]")
			}
			rows = append(rows, mutedStyle.Render(id)+"  "+baseStyle.Render(text)+tags)
		}
		s.logRaw(strings.Join(rows, "\n"))
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
	case "vision_config":
		var m map[string]json.RawMessage
		if json.Unmarshal(ev.Raw, &m) == nil {
			vm := map[string]bool{}
			if raw, ok := m["vision_models"]; ok {
				var arr []string
				if json.Unmarshal(raw, &arr) == nil {
					for _, id := range arr {
						vm[id] = true
					}
				}
			}
			s.visionModels = vm
			if raw, ok := m["vision_model"]; ok {
				var v string
				_ = json.Unmarshal(raw, &v)
				s.visionModel = v
			}
			if s.pendingVisionPicker {
				s.pendingVisionPicker = false
				s.openVisionPicker()
			}
		}
	case "skills":
		// Discoverable skills list (name + description + body content). Populates
		// the /skill:<name> command-palette entries; the body is inlined into the
		// apply_skill prompt by the core, so the TUI only needs name/desc here.
		var skills []skillInfo
		var m map[string]json.RawMessage
		if err := json.Unmarshal(ev.Raw, &m); err == nil {
			if raw, ok := m["skills"]; ok {
				_ = json.Unmarshal(raw, &skills)
			}
		}
		s.skillsList = skills
	}
	return waitForEvent(s.coreEvents)
}

// handleMouseWheel routes mouse-wheel events to the transcript viewport,
// mirroring handleScrollKey so follow mode stays consistent: scrolling up
// pauses follow (a streaming turn won't yank the view back to the bottom) and
// scrolling back to the bottom re-pins follow. Non-wheel mouse events (clicks
// and drags) are dropped so the wheel is the only mouse surface. Works in every
// state (idle/busy/approval) like the keyboard scroll bindings; modal overlays
// take over the whole screen and are skipped.
//
// Mouse tracking is opt-in: it's enabled at startup only when the Mouse Wheel
// setting is on, and toggled at runtime via /settings → Mouse Wheel
// applyModels sets the discovered model list and re-applies the persisted
// model selection + reasoning clamp. Shared by the `ready` and `models`
// events so a provider switch re-selects the same model id when present.
func (s *session) applyModels(models []modelInfo) {
	s.models = models
	s.modelIdx = 0
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
	if s.clampReasoning() {
		_ = s.settings.save()
		if s.modelIdx >= 0 && s.modelIdx < len(s.models) {
			s.logInfo(fmt.Sprintf("reasoning: %s (for %s)", s.settings.ReasoningEffort, s.models[s.modelIdx].ID))
		}
	}
}

// containsProvider reports whether name is in the core's configured provider list.
func (s *session) containsProvider(name string) bool {
	for _, p := range s.providers {
		if p == name {
			return true
		}
	}
	return false
}

// providerKey returns the persisted API key for a provider, preferring the
// per-provider key map over the legacy single APIKey (which applies to the
// default/active provider). Empty when nothing is stored.
func (s *session) providerKey(name string) string {
	if k, ok := s.settings.ProviderKeys[name]; ok && k != "" {
		return k
	}
	if s.settings.APIKey != "" {
		return s.settings.APIKey
	}
	return ""
}

// deleteProviderKey drops a provider's persisted key from the per-provider map
// (and the legacy single APIKey when it was the active/default provider). Used
// by /logout so the TUI side and the core agree the provider is logged out.
func (s *session) deleteProviderKey(name string) {
	if s.settings.ProviderKeys != nil {
		delete(s.settings.ProviderKeys, name)
	}
	if s.settings.APIKey != "" && (name == s.activeProvider || name == "default") {
		s.settings.APIKey = ""
	}
}

// sendProviderKey sends `set_key` for a named provider (or the active one when
// name is empty). Only sent when a key is actually available.
func (s *session) sendProviderKey(name string) bool {
	if name == "" {
		name = s.activeProvider
	}
	key := s.providerKey(name)
	if key == "" {
		return false
	}
	s.sendCore(map[string]any{"type": "set_key", "provider": name, "api_key": key})
	return true
}

// reauthActiveProvider re-sends the active provider's persisted key when the
// core reports it isn't authenticated yet (e.g. after launch or a switch to a
// provider whose key isn't in the config file/env).
func (s *session) reauthActiveProvider() {
	if s.providerHasKey {
		return // already authed for this provider
	}
	if s.sendProviderKey(s.activeProvider) {
		s.logInfo("sending key…")
	} else {
		s.logWarn("set your key: /key sk-...  (or open settings)")
	}
}

// (tea.EnableMouseCellMotion / tea.DisableMouse). Off (the default) leaves
// native click-drag text selection/copy to the terminal; when on, hold Shift
// to select/copy. Only wheel presses scroll; clicks and drags are ignored.
func (s *session) handleMouseWheel(msg tea.MouseMsg) tea.Cmd {
	// Modal overlays own the whole screen; never scroll the transcript behind one.
	if s.modal.kind != modalNone {
		return nil
	}
	// Only react to wheel presses; let clicks and drags fall through untouched.
	if msg.Action != tea.MouseActionPress {
		return nil
	}
	switch msg.Button {
	case tea.MouseButtonWheelUp:
		s.follow = false
		s.viewport.ScrollUp(s.viewport.MouseWheelDelta)
	case tea.MouseButtonWheelDown:
		s.viewport.ScrollDown(s.viewport.MouseWheelDelta)
		if s.viewport.AtBottom() {
			s.follow = true
		}
	}
	return nil
}

// sendSteer dispatches a steer command to the core: interrupt the running
// turn (if any) and redirect it with prompt. Marks a chained turn so the TUI
// keeps streaming across the interrupt. Used by both Ctrl+Enter and /steer.
// sendDelegation sends a prompt that instructs the orchestrator to invoke the
// subagent tool (the model applies the pi-subagents skill). cmdName is shown
// to the user as the originating slash command.
func (s *session) sendDelegation(prompt, cmdName string) tea.Cmd {
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
	s.logUser(prompt + "  ↳ " + cmdName)
	s.pushHistory(prompt)
	s.sendCore(s.withImages(map[string]any{
		"type":             "send",
		"prompt":           prompt,
		"model":            model,
		"reasoning_effort": s.settings.ReasoningEffort,
	}, prompt))
	s.busy = true
	return nil
}

// runSubagentCommand parses a /run, /parallel, or /chain slash command and
// delegates to the subagent tool via a structured prompt. Supported forms:
//
//	/run <agent> "<task>"            (single)
//	/parallel <a1> "<t1>" | <a2> "<t2>"   (parallel)
//	/chain <a1> "<t1>" -> <a2> "<t2>"      (chain, {previous} flows)
func (s *session) runSubagentCommand(parts []string, mode string) tea.Cmd {
	if len(parts) < 2 {
		usage := map[string]string{"single": "/run <agent> \"<task>\"", "parallel": "/parallel <a1> \"<t1>\" | <a2> \"<t2>\"", "chain": "/chain <a1> \"<t1>\" -> <a2> \"<t2>\""}
		s.logError("usage: " + usage[mode])
		return nil
	}
	rest := strings.TrimSpace(strings.Join(parts[1:], " "))
	var prompt string
	switch mode {
	case "single":
		agent, task := splitAgentTask(rest)
		if agent == "" {
			s.logError("usage: /run <agent> \"<task>\"")
			return nil
		}
		prompt = fmt.Sprintf("Run the subagent tool: agent=%q, task=%q. Return its result.", agent, task)
	case "parallel":
		tasks := splitParallel(rest)
		prompt = "Run the subagent tool in parallel mode with these tasks:\n" + tasks
	case "chain":
		steps := splitChain(rest)
		prompt = "Run the subagent tool as a chain with these steps (use {previous} to pass the prior step's output):\n" + steps
	}
	return s.sendDelegation(prompt, "/"+mode)
}

// splitAgentTask splits "agent \"task text\"" (or agent task...) into (agent, task).
func splitAgentTask(s string) (string, string) {
	s = strings.TrimSpace(s)
	if s == "" {
		return "", ""
	}
	// quoted task: agent "task"
	if idx := strings.IndexAny(s, "\""); idx >= 0 {
		agent := strings.TrimSpace(s[:idx])
		task := strings.Trim(s[idx:], "\"")
		return unquoteFirst(agent), strings.TrimSpace(task)
	}
	// bare: first token is agent, rest is task
	parts := strings.Fields(s)
	if len(parts) == 0 {
		return "", ""
	}
	return parts[0], strings.Join(parts[1:], " ")
}

func unquoteFirst(s string) string {
	s = strings.TrimSpace(s)
	if len(s) >= 2 && (s[0] == '"' && s[len(s)-1] == '"' || s[0] == '\'' && s[len(s)-1] == '\'') {
		return s[1 : len(s)-1]
	}
	return s
}

// splitParallel renders a parallel tasks list from "a1 \"t1\" | a2 \"t2\"".
func splitParallel(s string) string {
	var lines []string
	for i, step := range strings.Split(s, "|") {
		agent, task := splitAgentTask(step)
		if agent == "" {
			continue
		}
		lines = append(lines, fmt.Sprintf("  %d. agent=%q, task=%q", i+1, agent, task))
	}
	return strings.Join(lines, "\n")
}

// splitChain renders a chain steps list from "a1 \"t1\" -> a2 \"t2\"".
func splitChain(s string) string {
	var lines []string
	for i, step := range strings.Split(s, "->") {
		agent, task := splitAgentTask(step)
		if agent == "" {
			continue
		}
		if task == "" {
			lines = append(lines, fmt.Sprintf("  %d. agent=%q (task inherits {previous})", i+1, agent))
		} else {
			lines = append(lines, fmt.Sprintf("  %d. agent=%q, task=%q", i+1, agent, task))
		}
	}
	return strings.Join(lines, "\n")
}

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
	s.queued = &queuedMsg{kind: "steer", text: prompt, at: time.Now()}
	s.layout()
	s.sendCore(s.withImages(map[string]any{
		"type":             "steer",
		"prompt":           prompt,
		"model":            model,
		"reasoning_effort": s.settings.ReasoningEffort,
	}, prompt))
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
	s.queued = &queuedMsg{kind: "follow-up", text: text, at: time.Now()}
	s.layout()
	s.sendCore(s.withImages(map[string]any{
		"type":             "send",
		"prompt":           text,
		"model":            model,
		"reasoning_effort": s.settings.ReasoningEffort,
	}, text))
	return nil
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

func (s *session) handleKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	// global: the quit key (default Ctrl+C) quits unless a modal is open
	// (where esc / ctrl+c closes the modal instead).
	if s.kb(msg, "quit") && s.modal.kind == modalNone {
		if s.coreCmd != nil && s.coreCmd.Process != nil {
			_ = s.coreCmd.Process.Kill()
			_, _ = s.coreCmd.Process.Wait() // reap the core child so it isn't a zombie
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
	// global: toggle reasoning-block collapse/expand
	if s.kb(msg, "toggle_reasoning") {
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
	// global: toggle full output for the most recent tool block.
	// Tool output is truncated to the first 3 lines by default; this expands
	// (or collapses) the last tool call so the user can inspect full output.
	if s.kb(msg, "toggle_tool_output") {
		if b := s.lastToolOutputBlock(); b != nil {
			b.expanded = !b.expanded
			s.invalidateAll()
			s.refresh()
		}
		return s, nil
	}
	// global: open the command palette (default ctrl+p / ctrl+k).
	if s.kbAny(msg, "command_palette", "command_palette_alt") {
		s.openCommandPalette()
		return s, nil
	}
	// global: open the reasoning-effort picker for the active model.
	if s.kb(msg, "reasoning_picker") {
		s.openReasoningPicker()
		return s, nil
	}
	// "/" opens the palette when the input is empty — works while idle and
	// in-flight, mirroring the @-mention flyout (which also opens mid-turn).
	if msg.String() == "/" && s.input.Value() == "" {
		s.openCommandPalette()
		return s, nil
	}
	// @-mention flyout: when open it owns arrow/tab/enter/esc; printable
	// and editing keys fall through to the input and re-evaluate the token.
	if s.mentionActive && s.handleMentionNav(msg) {
		return s, nil
	}

	if s.pendingIntercom != nil {
		// A subagent asked the orchestrator a blocking question. Enter (the send
		// key) replies; Esc unblocks the child with a best-judgment nudge so it
		// isn't stuck.
		if s.kb(msg, "close") {
			s.sendCore(map[string]any{"type": "intercom_reply", "request_id": s.pendingIntercom.requestID, "reply": "[no reply — proceed with your best judgment]"})
			s.pendingIntercom = nil
			s.layout()
			s.input.Reset()
			s.input.Focus()
			return s, nil
		}
		if s.kb(msg, "send") {
			reply := strings.TrimSpace(s.input.Value())
			if reply == "" {
				// Empty intercom replies are never sent (Esc sends the
				// "[no reply]" nudge). Pulse the banner hint instead of a
				// silent no-op so the user knows to type — fixes
				// "Enter does not reply to the subagent".
				s.intercomNudge = time.Now()
				s.input.Focus()
				return s, nil
			}
			s.sendCore(map[string]any{"type": "intercom_reply", "request_id": s.pendingIntercom.requestID, "reply": reply})
			s.logSuccess(fmt.Sprintf("↦ reply to %s sent", s.pendingIntercom.from))
			s.pendingIntercom = nil
			s.input.Reset()
			s.layout()
			return s, nil
		}
		var cmd tea.Cmd
		s.input, cmd = s.input.Update(msg)
		return s, cmd
	}
	if s.pendingApproval != nil {
		switch {
		case s.kb(msg, "approve"):
			s.sendCore(map[string]any{"type": "approve", "request_id": s.pendingApproval.requestID, "decision": "yes"})
			s.pendingApproval = nil
		case s.kb(msg, "approve_always"):
			s.sendCore(map[string]any{"type": "approve", "request_id": s.pendingApproval.requestID, "decision": "always"})
			s.pendingApproval = nil
		case s.kbAny(msg, "deny", "close"):
			s.sendCore(map[string]any{"type": "approve", "request_id": s.pendingApproval.requestID, "decision": "no"})
			s.logError("denied")
			s.pendingApproval = nil
		default:
			// P2-17: pass non-decision keys to the input so typing isn't swallowed
			// while an approval banner is up (and the user can't accidentally
			// approve by typing "y...").
			var cmd tea.Cmd
			s.input, cmd = s.input.Update(msg)
			return s, cmd
		}
		s.layout() // banner released, grow viewport back
		return s, nil
	}
	if s.busy {
		// While a turn runs the input stays live: type a follow-up (Enter),
		// steer the model (Ctrl+Enter), run a slash command, or abort (Esc).
		// Scrolling + ctrl+t/o/p above still work; a lone "/" with an empty
		// input opens the command palette mid-turn too (like the @ flyout).
		switch {
		case s.kb(msg, "close"):
			// Esc peels off layers: if a follow-up/steer is queued, first Esc just
			// drops the queued message (the in-flight turn keeps running); a
			// second Esc then aborts the running turn. This matches the user's
			// mental model of "cancel the queued thing" without nuking the
			// whole in-flight chat.
			if s.queued != nil {
				kind := s.queued.kind
				s.queued = nil
				s.queuedNext = false
				s.sendCore(map[string]any{"type": "clear_queue"})
				s.layout() // release the queue banner
				if kind == "steer" {
					s.logInfo("steer cancelled (the running turn was already interrupted)")
				} else {
					s.logInfo("queued follow-up cancelled — turn continues")
				}
				return s, nil
			}
			s.queuedNext = false
			s.sendCore(map[string]any{"type": "abort"})
			s.logWarn("aborting…")
			return s, nil
		case s.kb(msg, "steer"):
			return s, s.steerFromInput()
		case s.kb(msg, "send"):
			text := strings.TrimSpace(s.input.Value())
			if text == "" {
				return s, nil
			}
			s.input.Reset()
			s.evalMention()
			if strings.HasPrefix(text, "/") {
				return s, s.handleUserLine(text)
			}
			return s, s.queueFollowUp(text)
		}
		var cmd tea.Cmd
		s.input, cmd = s.input.Update(msg)
		s.evalMention()
		return s, cmd
	}

	// welcome-screen navigation: when the conversation is empty, ↑/↓ move the
	// example cursor; enter drops the selected example into the (editable)
	// input. Only arrow keys are used so typing letters/digits is unaffected.
	if len(s.blocks) == 0 && s.pendingApproval == nil {
		switch {
		case s.kb(msg, "nav_up"):
			s.welcomeIdx = (s.welcomeIdx - 1 + len(welcomeExamples)) % len(welcomeExamples)
			return s, nil
		case s.kb(msg, "nav_down"):
			s.welcomeIdx = (s.welcomeIdx + 1) % len(welcomeExamples)
			return s, nil
		}
		if s.kb(msg, "select") && strings.TrimSpace(s.input.Value()) == "" {
			s.input.SetValue(welcomeExamples[s.welcomeIdx])
			s.evalMention()
			s.input.Focus()
			return s, nil
		}
	}

	// history recall: up/down when the input is focused and not empty-positioned
	if s.kb(msg, "history_prev") && len(s.history) > 0 {
		val := s.recallHistory(-1)
		s.input.SetValue(val)
		s.evalMention()
		return s, nil
	}
	if s.kb(msg, "history_next") && len(s.history) > 0 {
		val := s.recallHistory(+1)
		s.input.SetValue(val)
		s.evalMention()
		return s, nil
	}

	if s.kb(msg, "send") {
		text := strings.TrimSpace(s.input.Value())
		if text == "" {
			return s, nil
		}
		s.input.Reset()
		s.evalMention()
		s.histIdx = len(s.history)
		return s, s.handleUserLine(text)
	}

	s.input, _ = s.input.Update(msg)
	s.evalMention()
	return s, nil
}

// handleScrollKey moves the transcript viewport and manages follow mode.
// Returns true when it consumed the key. Scroll-up motions pause follow (so the
// view isn't yanked to the bottom on the next token); scroll-down re-pins
// follow once the bottom is reached.
func (s *session) handleScrollKey(msg tea.KeyMsg) bool {
	switch {
	case s.kb(msg, "scroll_page_up"):
		s.follow = false
		s.viewport.PageUp()
		return true
	case s.kb(msg, "scroll_page_down"):
		s.viewport.PageDown()
		if s.viewport.AtBottom() {
			s.follow = true
		}
		return true
	case s.kb(msg, "scroll_line_up"):
		s.follow = false
		s.viewport.LineUp(1)
		return true
	case s.kb(msg, "scroll_line_down"):
		s.viewport.LineDown(1)
		if s.viewport.AtBottom() {
			s.follow = true
		}
		return true
	case s.kb(msg, "scroll_top"):
		s.follow = false
		s.viewport.GotoTop()
		return true
	case s.kb(msg, "scroll_bottom"):
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
		// /skill:<name> [optional task] — invoke a discoverable skill. Handled
		// before the switch because the command token is dynamic (/skill:<x>
		// has no fixed case). The core reads the SKILL.md and runs the turn.
		if strings.HasPrefix(parts[0], "/skill:") {
			return s.handleSkillCommand(parts)
		}
		switch parts[0] {
		case "/login":
			s.openLoginPicker()
			return nil
		case "/logout":
			if len(parts) >= 2 {
				// /logout <provider> — direct logout without the picker.
				name := parts[1]
				s.sendCore(map[string]any{"type": "logout", "provider": name})
				s.deleteProviderKey(name)
				if s.settings.ActiveProvider == name {
					s.settings.ActiveProvider = ""
				}
				_ = s.settings.save()
				s.logInfo("logged out of " + name)
				return nil
			}
			s.openLogoutPicker()
			return nil
		case "/key":
			// /key <value> sets the API key for the active provider. Kept as a
			// convenience because the app's "not authenticated" errors direct the
			// user here ("run /key sk-... first"); the full multi-provider flow is
			// /login. With no argument, /key opens the settings modal on the API
			// Key field so the user can paste one inline.
			if len(parts) < 2 {
				s.openSettings()
				s.modal.fieldIdx = s.settingsFieldIndex("API Key")
				if s.modal.fieldIdx < 0 {
					s.modal.fieldIdx = 0
				}
				s.logInfo("paste your key in the API Key field, then Enter")
				return nil
			}
			key := parts[1]
			// Scope the key to the active provider (per-provider keys). Also keep the
			// legacy single APIKey field in sync for the default/active provider.
			if s.activeProvider == "" {
				s.activeProvider = "default"
			}
			if s.settings.ProviderKeys == nil {
				s.settings.ProviderKeys = map[string]string{}
			}
			s.settings.ProviderKeys[s.activeProvider] = key
			s.settings.APIKey = key
			_ = s.settings.save()
			s.sendCore(map[string]any{"type": "set_key", "provider": s.activeProvider, "api_key": key})
			s.logInfo(fmt.Sprintf("sending key for provider '%s'…", s.activeProvider))
			return nil
		case "/oauth-code":
			// /oauth-code [code] completes a pending no-browser OAuth login (the
			// SSH/headless Google flow). With an inline code it sends immediately;
			// with no argument it opens a modal to paste the code into — the long
			// Google code is awkward to paste inline after the command (the command
			// input mangles/truncates it).
			if len(parts) >= 2 {
				code := strings.Join(parts[1:], " ")
				s.sendCore(map[string]any{"type": "oauth_code", "code": code})
				s.logInfo("submitting OAuth code…")
				return nil
			}
			s.openOauthCodeModal()
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
			s.queued = nil
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
		case "/keybinds":
			s.openKeybindsModal()
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
			// P2-12: validate the image like the main send paths do (via withImages),
			// so /attach can't base64-encode a non-image or a >20MiB file (it
			// previously set "images" directly with no checks).
			abs, err := validateImage(imgPath)
			if err != nil {
				s.logError(err.Error())
				return nil
			}
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
			s.sendCore(s.withImages(map[string]any{
				"type":             "send",
				"prompt":           promptText,
				"model":            model,
				"reasoning_effort": s.settings.ReasoningEffort,
				"images":           []string{abs},
			}, promptText))
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
		case "/remember":
			rest := strings.TrimSpace(strings.Join(parts[1:], " "))
			if rest == "" {
				s.logError("usage: /remember <text>")
				return nil
			}
			s.sendCore(map[string]any{"type": "save_memory", "text": rest})
			s.sendCore(map[string]any{"type": "refresh_memory"})
			s.logSuccess("memory saved")
			return nil
		case "/memory":
			s.sendCore(map[string]any{"type": "list_memory"})
			return nil
		case "/forget":
			if len(parts) < 2 {
				s.logError("usage: /forget <id>")
				return nil
			}
			s.sendCore(map[string]any{"type": "forget_memory", "id": parts[1]})
			return nil
		case "/index":
			// Bootstrap learning on this repo: walk the structure and persist durable
			// knowledge as memories + note candidate skills. Pure delegation to the
			// orchestrator (it has read/grep/glob/bash + the memory tool); no core
			// command needed. --full re-indexes from scratch; --incremental only
			// covers files changed since the last index (detected via git).
			mode := "full"
			for _, a := range parts[1:] {
				if a == "--incremental" || a == "-i" {
					mode = "incremental"
				} else if a == "--full" || a == "-f" {
					mode = "full"
				}
			}
			var task string
			if mode == "incremental" {
				task = "Run an incremental knowledge index of this repository. Use `git status` + `git diff --name-only` to find files changed since the last index; for each changed area, read it and use the `memory` tool (action: append) to UPDATE the relevant existing memories — architecture, conventions, APIs, gotchas — rather than creating duplicates. If a changed file reveals a new subsystem with no memory yet, save a new one. Then list the memories you touched. Be concise: only persist what genuinely changed."
			} else {
				task = "Run a full knowledge index of this repository to bootstrap learning. Walk the top-level layout, read README/package-manifest/entry points/config/tests, and identify the architecture, major subsystems, conventions, reusable patterns, build/test/deploy steps, and gotchas. Use the `memory` tool (action: save) to persist each as a durable, named memory (types: architecture/convention/api/gotcha/build). Then use `list_dir .catalyst-code/skills/` and, for any reusable workflow you solved 2+ times that has no skill yet, write a candidate SKILL.md under `.catalyst-code/skills/<name>/` with write_file (frontmatter: name/description; body: when-to-use + steps + example). End by listing the memories and any candidate skills you created, and name one area you are least confident about."
			}
			return s.sendDelegation(task, "/index")
		case "/reflect":
			// Deliberate end-of-task learning pass: critique the recent work in this
			// session and persist durable takeaways via the memory tool. Pure
			// delegation — no core command needed.
			task := "Reflect on the work done in this session so far. Identify: (1) any convention, architecture fact, decision, or gotcha worth persisting so future sessions don't rediscover it, and (2) any repetitive pattern you performed more than once that should become a reusable skill under `.catalyst-code/skills/`. Use the `memory` tool (action: append if a topic memory exists, else save) to persist durable facts only — skip transient task state. If you wrote a skill, name it. Finish with a two-line summary: what you learned and what you persisted."
			return s.sendDelegation(task, "/reflect")
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
		case "/plugin-list", "/plugin-config":
			s.sendCore(map[string]any{"type": "list_plugins"})
			return nil
		case "/vision":
			s.pendingVisionPicker = true
			s.sendCore(map[string]any{"type": "get_vision_config"})
			s.logInfo("loading vision config…")
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
		case "/run":
			return s.runSubagentCommand(parts, "single")
		case "/parallel":
			return s.runSubagentCommand(parts, "parallel")
		case "/chain":
			return s.runSubagentCommand(parts, "chain")
		case "/subagents", "/subagents-list":
			return s.sendDelegation(`Run subagent({ action: "list" }) and show the available agents.`, "/subagents")
		case "/subagents-doctor":
			return s.sendDelegation(`Run subagent({ action: "doctor" }) and show the setup diagnostics.`, "/subagents-doctor")
		case "/subagents-status":
			return s.sendDelegation(`Run subagent({ action: "status" }) and show the active subagent runs.`, "/subagents-status")
		case "/subagents-models":
			return s.sendDelegation(`Run subagent({ action: "models" }) and show the runtime model mapping for the builtin agents.`, "/subagents-models")
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
	s.sendCore(s.withImages(map[string]any{
		"type":             "send",
		"prompt":           text,
		"model":            model,
		"reasoning_effort": s.settings.ReasoningEffort,
	}, text))
	s.busy = true
	return nil
}

// handleSkillCommand dispatches "/skill:<name> [optional task]". It resolves the
// skill from the cached skills list and sends an apply_skill command to the
// core, which reads the SKILL.md (resolving project > user scope, bypassing
// read_file's path restriction so global skills work too) and runs a turn that
// applies it. The displayed user line is the concise "/skill:<name> [task]";
// the full skill body is injected by the core, not shown in the transcript.
func (s *session) handleSkillCommand(parts []string) tea.Cmd {
	token := parts[0] // "/skill:<name>"
	name := strings.TrimPrefix(token, "/skill:")
	if name == "" {
		s.logError("usage: /skill:<name> [optional task]")
		return nil
	}
	var found *skillInfo
	for i := range s.skillsList {
		if strings.EqualFold(s.skillsList[i].Name, name) {
			found = &s.skillsList[i]
			break
		}
	}
	if found == nil {
		s.logError("unknown skill: " + name)
		return nil
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
	task := strings.TrimSpace(strings.Join(parts[1:], " "))
	display := token
	if task != "" {
		display = token + " " + task
	}
	s.follow = true
	s.logUser(display)
	s.pushHistory(display)
	cmd := map[string]any{
		"type":             "apply_skill",
		"name":             found.Name,
		"model":            model,
		"reasoning_effort": s.settings.ReasoningEffort,
	}
	if task != "" {
		cmd["task"] = task
	}
	s.sendCore(cmd)
	s.busy = true
	return nil
}

// openURL best-effort opens a URL in the OS default browser (used to surface an
// OAuth login URL). Errors are ignored — the URL is also shown in the transcript.
func openURL(url string) {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		cmd = exec.Command("open", url)
	case "windows":
		cmd = exec.Command("cmd", "/C", "start", "", url)
	default:
		cmd = exec.Command("xdg-open", url)
	}
	cmd.Stdin = nil
	cmd.Stdout = nil
	cmd.Stderr = nil
	_ = cmd.Start()
}

// copyToClipboardOSC52 writes the OSC 52 escape sequence to set the LOCAL
// terminal's clipboard to text. Over SSH the sequence passes through to the
// user's local terminal, which writes its clipboard — so the user can paste
// (Ctrl/Cmd+V) into their local browser without copying from the (wrapped,
// hard-to-select) transcript. Best-effort: terminals that don't support OSC 52
// ignore it. The sequence is invisible (no cursor move / no text), so it is
// safe to emit from a Bubble Tea handler between render frames.
func copyToClipboardOSC52(text string) {
	if text == "" {
		return
	}
	// OSC 52: ESC ] 52 ; <selection> ; <base64> BEL.  'c' = the CLIPBOARD
	// selection (the Ctrl/Cmd+V paste buffer).
	enc := base64.StdEncoding.EncodeToString([]byte(text))
	os.Stdout.WriteString("\x1b]52;c;" + enc + "\x07")
}
