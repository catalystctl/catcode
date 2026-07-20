package main

import (
	"testing"
	"time"

	"charm.land/bubbles/v2/list"
	tea "charm.land/bubbletea/v2"
)

// pumpPickerKey sends a key through handlePickerListKey and drains any
// immediate FilterMatchesMsg (and similar) cmds so the list filter state
// matches what a live Bubble Tea loop would see. Expands tea.BatchMsg so a
// Batch'd filterItems+Blink is applied instead of being ignored by list.Update.
func pumpPickerKey(s *session, msg tea.KeyPressMsg) {
	_, cmd := s.handlePickerListKey(msg)
	drainPickerCmds(s, cmd)
}

func drainPickerCmds(s *session, cmd tea.Cmd) {
	queue := []tea.Cmd{cmd}
	for i := 0; i < 16 && len(queue) > 0; i++ {
		c := queue[0]
		queue = queue[1:]
		if c == nil {
			continue
		}
		ch := make(chan tea.Msg, 1)
		go func(c tea.Cmd) { ch <- c() }(c)
		var next tea.Msg
		select {
		case next = <-ch:
		case <-time.After(20 * time.Millisecond):
			continue // Blink/Tick — skip
		}
		switch m := next.(type) {
		case tea.BatchMsg:
			queue = append(queue, []tea.Cmd(m)...)
		default:
			var follow tea.Cmd
			s.modal.pickerList, follow = s.modal.pickerList.Update(m)
			if follow != nil {
				queue = append(queue, follow)
			}
		}
	}
}

// TestPickerListEnterWhileFilteringAppliesFilter: bubbles/list "/" filter mode
// must receive Enter so it can AcceptWhileFiltering. Before the fix, our select
// intercept ran first and executed the first list item instead of applying the
// typed filter.
func TestPickerListEnterWhileFilteringAppliesFilter(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.authed = true
	s.width, s.height = 80, 24
	s.models = []modelInfo{{ID: "m1", Name: "Model 1"}}
	s.modelIdx = 0
	s.openCommandPalette()

	if s.modal.kind != modalCommand {
		t.Fatalf("precondition: palette open; kind=%v", s.modal.kind)
	}

	pumpPickerKey(s, keyMsg("/"))
	if !s.modal.pickerList.SettingFilter() {
		t.Fatal("\"/\" should enter bubbles/list filter mode")
	}

	// Filter for something that is NOT the first palette entry, so a mistaken
	// select-on-enter would leave modalCommand (and fail the FilterApplied check).
	for _, r := range "theme" {
		pumpPickerKey(s, keyMsg(string(r)))
	}
	if got := s.modal.pickerList.FilterValue(); got != "theme" {
		t.Fatalf("filter value = %q, want theme", got)
	}
	if !s.modal.pickerList.SettingFilter() {
		t.Fatal("should still be SettingFilter after typing")
	}

	pumpPickerKey(s, tea.KeyPressMsg{Code: tea.KeyEnter})

	if s.modal.kind != modalCommand {
		t.Fatalf("Enter while filtering must NOT select; modal kind=%v (want modalCommand)", s.modal.kind)
	}
	if s.modal.pickerList.SettingFilter() {
		t.Fatal("Enter should leave SettingFilter (accept the filter)")
	}
	if s.modal.pickerList.FilterState() != list.FilterApplied {
		t.Fatalf("filter state=%v, want FilterApplied", s.modal.pickerList.FilterState())
	}
	if got := s.modal.pickerList.FilterValue(); got != "theme" {
		t.Fatalf("applied filter value = %q, want theme", got)
	}
}

// TestPickerListEscWhileFilteringCancelsFilter: Esc during "/" filter editing
// must cancel the filter, not close the picker modal.
func TestPickerListEscWhileFilteringCancelsFilter(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.authed = true
	s.width, s.height = 80, 24
	s.models = []modelInfo{{ID: "m1", Name: "Model 1"}}
	s.modelIdx = 0
	s.openCommandPalette()

	pumpPickerKey(s, keyMsg("/"))
	pumpPickerKey(s, keyMsg("t"))
	if !s.modal.pickerList.SettingFilter() {
		t.Fatal("precondition: SettingFilter")
	}

	pumpPickerKey(s, tea.KeyPressMsg{Code: tea.KeyEscape})

	if s.modal.kind != modalCommand {
		t.Fatalf("Esc while filtering must keep palette open; kind=%v", s.modal.kind)
	}
	if s.modal.pickerList.SettingFilter() {
		t.Fatal("Esc should cancel SettingFilter")
	}
	if s.modal.pickerList.FilterState() != list.Unfiltered {
		t.Fatalf("filter state=%v, want Unfiltered", s.modal.pickerList.FilterState())
	}
}

// TestPickerListTypingShrinksVisibleItems: while SettingFilter, each keystroke
// must update filteredItems (via FilterMatchesMsg) so the visible list shrinks.
// The live TUI depends on session.Update forwarding FilterMatchesMsg into
// pickerList — without that, filteredItems stays stale and typing does nothing.
func TestPickerListTypingShrinksVisibleItems(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.authed = true
	s.width, s.height = 80, 24
	s.models = []modelInfo{{ID: "m1", Name: "Model 1"}}
	s.modelIdx = 0
	s.openCommandPalette()

	before := len(s.modal.pickerList.VisibleItems())
	if before < 2 {
		t.Fatalf("precondition: need multiple palette items, got %d", before)
	}

	pumpPickerKey(s, keyMsg("/"))
	for _, r := range "new" {
		pumpPickerKey(s, keyMsg(string(r)))
	}
	if got := s.modal.pickerList.FilterValue(); got != "new" {
		t.Fatalf("filter value = %q, want new", got)
	}
	after := len(s.modal.pickerList.VisibleItems())
	if after >= before {
		t.Fatalf("typing filter should shrink list; before=%d after=%d", before, after)
	}
	if after == 0 {
		t.Fatal("filter \"new\" should still match /new")
	}
}

// TestUpdateForwardsFilterMatchesMsgToPicker: session.Update must not drop
// list.FilterMatchesMsg while a Charm picker is open — that was why typing
// updated the filter input but left the visible rows unchanged.
func TestUpdateForwardsFilterMatchesMsgToPicker(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.authed = true
	s.width, s.height = 80, 24
	s.models = []modelInfo{{ID: "m1", Name: "Model 1"}}
	s.modelIdx = 0
	s.openCommandPalette()

	before := len(s.modal.pickerList.VisibleItems())

	// Enter filter mode and type via list.Update (not pump) so we capture the
	// real filterItems cmd the TUI returns to the program loop.
	var cmd tea.Cmd
	s.modal.pickerList, cmd = s.modal.pickerList.Update(keyMsg("/"))
	_ = cmd // Blink — do not block on Tick
	if !s.modal.pickerList.SettingFilter() {
		t.Fatal("\"/\" should enter SettingFilter")
	}
	s.modal.pickerList, cmd = s.modal.pickerList.Update(keyMsg("n"))
	s.modal.pickerList, cmd = s.modal.pickerList.Update(keyMsg("e"))
	s.modal.pickerList, cmd = s.modal.pickerList.Update(keyMsg("w"))
	if cmd == nil {
		t.Fatal("typing while filtering should return a filterItems cmd")
	}

	fm := takeFilterMatchesMsg(t, cmd)
	_, follow := s.Update(fm)
	_ = follow

	after := len(s.modal.pickerList.VisibleItems())
	if after >= before {
		t.Fatalf("Update(FilterMatchesMsg) should shrink list; before=%d after=%d", before, after)
	}
	if s.modal.pickerList.FilterValue() != "new" {
		t.Fatalf("filter value = %q, want new", s.modal.pickerList.FilterValue())
	}
}

// TestPickerListTypeToFilter: typing a printable key while NOT in filter mode
// must jump straight into filtering (the pre-Charm modal behavior) — no "/"
// prefix required. The list must shrink on the same keystrokes.
func TestPickerListTypeToFilter(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.authed = true
	s.width, s.height = 80, 24
	s.models = []modelInfo{{ID: "m1", Name: "Model 1"}}
	s.modelIdx = 0
	s.openCommandPalette()

	before := len(s.modal.pickerList.VisibleItems())
	if before < 2 {
		t.Fatalf("precondition: need multiple palette items, got %d", before)
	}

	for _, r := range "theme" {
		pumpPickerKey(s, keyMsg(string(r)))
	}
	if !s.modal.pickerList.SettingFilter() {
		t.Fatal("plain typing should enter filter mode without a leading \"/\"")
	}
	if got := s.modal.pickerList.FilterValue(); got != "theme" {
		t.Fatalf("filter value = %q, want theme", got)
	}
	after := len(s.modal.pickerList.VisibleItems())
	if after >= before {
		t.Fatalf("type-to-filter should shrink list; before=%d after=%d", before, after)
	}
	if after == 0 {
		t.Fatal("filter \"theme\" should still match /theme")
	}
}

// TestPickerListSlashStillOpensEmptyFilter: "/" must keep its native behavior —
// enter filter mode without inserting a literal "/" into the filter input.
func TestPickerListSlashStillOpensEmptyFilter(t *testing.T) {
	s := initialSession()
	s.ready = true
	s.authed = true
	s.width, s.height = 80, 24
	s.models = []modelInfo{{ID: "m1", Name: "Model 1"}}
	s.modelIdx = 0
	s.openCommandPalette()

	pumpPickerKey(s, keyMsg("/"))
	if !s.modal.pickerList.SettingFilter() {
		t.Fatal("\"/\" should enter filter mode")
	}
	if got := s.modal.pickerList.FilterValue(); got != "" {
		t.Fatalf("filter value = %q, want empty (slash must not be inserted)", got)
	}
}

// takeFilterMatchesMsg runs cmd (expanding BatchMsg) and returns the first
// FilterMatchesMsg. Skips slow Tick/Blink cmds via a short timeout.
func takeFilterMatchesMsg(t *testing.T, cmd tea.Cmd) list.FilterMatchesMsg {
	t.Helper()
	if cmd == nil {
		t.Fatal("nil cmd")
	}
	queue := []tea.Cmd{cmd}
	for len(queue) > 0 {
		c := queue[0]
		queue = queue[1:]
		if c == nil {
			continue
		}
		ch := make(chan tea.Msg, 1)
		go func(c tea.Cmd) { ch <- c() }(c)
		select {
		case msg := <-ch:
			switch m := msg.(type) {
			case list.FilterMatchesMsg:
				return m
			case tea.BatchMsg:
				queue = append(queue, []tea.Cmd(m)...)
			default:
				// Blink result or other — ignore for this helper
			}
		case <-time.After(20 * time.Millisecond):
			// Likely tea.Tick (cursor blink) — leave it
		}
	}
	t.Fatal("no FilterMatchesMsg in cmd")
	return nil
}
