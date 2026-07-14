package main

import (
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
)

// ---------------------------------------------------------------------------
// @-mention file flyout
//
// Typing "@" in the chat input opens a flyout above the input box that lists
// files/directories. The list refines as the user keeps typing the path query.
// Arrow keys move the selection, Tab/Enter accepts it (replacing the @-token
// with the chosen path), Esc closes it.
//
// The query is the text between the last unbroken "@" and the cursor. A "/"
// in the query switches to directory-listing completion; a bare query (no
// slash) does a recursive name search under the workspace CWD. Mentions never
// escape that boundary, including through absolute paths or ".." traversal.
// ---------------------------------------------------------------------------

const mentionMaxResults = 12
const mentionMaxVisible = 8

type mentionItem struct {
	display string // text shown in the flyout (dirs get a trailing "/")
	insert  string // path inserted after "@" on accept
	isDir   bool
}

type mentionSearchState uint8

const (
	mentionSearchReady mentionSearchState = iota
	mentionSearchLoading
	mentionSearchFailed
)

// mentionSearchMsg wakes Bubble Tea when the asynchronous repository walk
// finishes. The cwd is carried so a late result from a previous directory can
// never overwrite the flyout for the current one.
type mentionSearchMsg struct {
	cwd string
}

// mentionAt is the rune index of the active "@" in the input; mentionScroll
// is the flyout's vertical scroll offset for long lists.
func (s *session) closeMention() {
	s.mentionActive = false
	s.mentionItems = nil
	s.mentionCursor = 0
	s.mentionScroll = 0
}

// evalMention inspects the input at the cursor and activates/refreshes the
// flyout when an @-token is present, or closes it otherwise. Called after
// every keystroke that mutates the input.
func (s *session) evalMention() tea.Cmd {
	s.mentionActive = false
	s.mentionItems = nil
	runes := []rune(s.input.Value())
	pos := s.input.Position()
	if pos <= 0 || pos > len(runes) {
		s.mentionCursor = 0
		s.mentionScroll = 0
		return nil
	}
	// Walk back from the cursor for an "@" with no whitespace between it and
	// the cursor — that span is the mention query.
	at := -1
	for i := pos - 1; i >= 0; i-- {
		r := runes[i]
		if r == ' ' || r == '\t' || r == '\n' {
			break
		}
		if r == '@' {
			at = i
			break
		}
	}
	if at < 0 {
		s.mentionCursor = 0
		s.mentionScroll = 0
		return nil
	}
	// Require a word boundary before "@" so emails / "foo@bar" don't trigger.
	if at > 0 {
		prev := runes[at-1]
		if prev != ' ' && prev != '\t' && prev != '\n' {
			s.mentionCursor = 0
			s.mentionScroll = 0
			return nil
		}
	}
	query := string(runes[at+1 : pos])
	s.mentionAt = at
	s.mentionActive = true
	var refresh tea.Cmd
	if query != "" && !strings.Contains(query, "/") {
		s.mentionItems, refresh = recursiveSearchWithRefresh(query)
	} else {
		s.mentionItems = dirCompletion(query)
	}
	if s.mentionCursor >= len(s.mentionItems) {
		s.mentionCursor = 0
	}
	if s.mentionCursor < 0 {
		s.mentionCursor = 0
	}
	s.mentionScroll = 0
	return refresh
}

// handleMentionSearchMsg refreshes an open flyout after its background walk.
// Callers should route mentionSearchMsg through session.Update and return this
// command (normally nil; it is non-nil only if the cwd changed concurrently).
func (s *session) handleMentionSearchMsg(msg mentionSearchMsg) tea.Cmd {
	if !s.mentionActive {
		return nil
	}
	cwd, err := os.Getwd()
	if err != nil || cwd != msg.cwd {
		return nil
	}
	return s.evalMention()
}

// handleMentionNav owns arrow/tab/enter/esc while the flyout is open. Returns
// true when it consumed the key. Printable/editing keys return false so they
// flow into the input (and then evalMention re-runs).
func (s *session) handleMentionNav(msg tea.KeyPressMsg) bool {
	if !s.mentionActive {
		return false
	}
	n := len(s.mentionItems)
	switch {
	case s.kbAny(msg, "nav_up", "nav_up_alt"):
		if n > 0 {
			s.mentionCursor = (s.mentionCursor - 1 + n) % n
		}
		return true
	case s.kbAny(msg, "nav_down", "nav_down_alt"):
		if n > 0 {
			s.mentionCursor = (s.mentionCursor + 1) % n
		}
		return true
	case s.kb(msg, "mention_accept"):
		if n > 0 {
			s.acceptMention()
		}
		return true
	case s.kb(msg, "select"):
		// With matches, Enter accepts; with no matches, fall through so the
		// message can be sent as-is.
		if n > 0 {
			s.acceptMention()
			return true
		}
		return false
	case s.kb(msg, "close"):
		s.closeMention()
		return true
	}
	return false
}

// acceptMention replaces the @-token in the input with the selected path.
// Selecting a directory keeps the flyout open (drill-in); selecting a file
// closes it and adds a trailing space so the token doesn't re-trigger.
func (s *session) acceptMention() {
	if !s.mentionActive || len(s.mentionItems) == 0 {
		return
	}
	it := s.mentionItems[s.mentionCursor]
	runes := []rune(s.input.Value())
	at := s.mentionAt // rune index of "@"
	pos := s.input.Position()
	if at < 0 || at > len(runes) || pos < at || pos > len(runes) {
		return
	}
	insertRunes := []rune(it.insert)
	suffix := []rune("")
	if !it.isDir {
		suffix = []rune(" ")
	}
	var out []rune
	out = append(out, runes[:at]...)
	out = append(out, '@')
	out = append(out, insertRunes...)
	out = append(out, suffix...)
	out = append(out, runes[pos:]...)
	s.input.SetValue(string(out))
	s.input.SetCursor(at + 1 + len(insertRunes) + len(suffix))
	if it.isDir {
		s.evalMention() // reopen listing for the newly-typed directory
	} else {
		s.closeMention()
	}
}

// ---------------------------------------------------------------------------
// File search
// ---------------------------------------------------------------------------

// computeMentionItems builds the candidate list for a query. A query
// containing "/" (or empty, meaning a bare "@") uses directory-listing
// completion; otherwise a recursive name search under the CWD.
func computeMentionItems(query string) []mentionItem {
	if query == "" || strings.Contains(query, "/") {
		return dirCompletion(query)
	}
	return recursiveSearch(query)
}

// dirCompletion lists one directory and filters by the final path component.
func dirCompletion(query string) []mentionItem {
	cwd, err := os.Getwd()
	if err != nil || filepath.IsAbs(filepath.FromSlash(query)) {
		return nil
	}
	var dirPart, prefix string
	if i := strings.LastIndex(query, "/"); i >= 0 {
		dirPart = query[:i]
		prefix = query[i+1:]
	} else {
		// Bare "@" or no slash: list the CWD itself.
		dirPart = "."
		prefix = query
	}
	base := filepath.Clean(filepath.Join(cwd, filepath.FromSlash(dirPart)))
	if !withinWorkspace(cwd, base) {
		return nil
	}
	entries, err := os.ReadDir(base)
	if err != nil {
		return nil
	}
	lp := strings.ToLower(prefix)
	showHidden := strings.HasPrefix(prefix, ".")
	var items []mentionItem
	for _, e := range entries {
		name := e.Name()
		if !showHidden && strings.HasPrefix(name, ".") {
			continue
		}
		if !strings.HasPrefix(strings.ToLower(name), lp) {
			continue
		}
		isDir := e.IsDir()
		disp := name
		ins := name
		if dirPart != "." && dirPart != "" {
			ins = strings.TrimSuffix(filepath.ToSlash(dirPart), "/") + "/" + name
		}
		if isDir {
			disp += "/"
			ins += "/"
		}
		items = append(items, mentionItem{display: disp, insert: ins, isDir: isDir})
		if len(items) >= mentionMaxResults {
			break
		}
	}
	sortMentionItems(items)
	return items
}

func withinWorkspace(root, path string) bool {
	rel, err := filepath.Rel(root, path)
	return err == nil && rel != ".." && !strings.HasPrefix(rel, ".."+string(filepath.Separator))
}

// mentionCache memoizes the recursive file list for the CWD so a bare `@query`
// doesn't re-walk the whole tree (up to 40k entries) on every keystroke (P1-18).
// The walk is the expensive part (40k stat calls); filtering the cached list by
// prefix is microseconds. A short TTL keeps it fresh as files are added.
var mentionCache = struct {
	sync.Mutex
	cwd     string
	list    []mentionItem
	at      time.Time
	walking bool // a background walk for `cwd` is in flight (prevents duplicate/clobbering walks)
	done    chan struct{}
	err     error
}{}

const mentionCacheTTL = 2 * time.Second
const mentionCacheCap = 10000

// recursiveSearch returns files under the CWD whose path contains the prefix.
// The expensive walk (up to 40k stat calls) runs in a BACKGROUND goroutine so
// it never freezes the UI thread — evalMention reads the cached list and
// returns an empty result while the first walk is in flight. The `walking` flag
// guarantees only one walk per cwd at a time (no duplicate or clobbering fills).
func recursiveSearch(prefix string) []mentionItem {
	items, _ := recursiveSearchWithRefresh(prefix)
	return items
}

// recursiveSearchWithRefresh returns currently cached matches and, while a
// walk is in flight, a command that emits mentionSearchMsg as soon as it
// finishes. Returning a Tea command is what makes the flyout redraw without
// requiring the user to press another key.
func recursiveSearchWithRefresh(prefix string) ([]mentionItem, tea.Cmd) {
	cwd, err := os.Getwd()
	if err != nil {
		return nil, nil
	}
	mentionCache.Lock()
	needWalk := false
	if mentionCache.cwd != cwd || time.Since(mentionCache.at) > mentionCacheTTL {
		// Stale or missing: kick a background walk unless one is already running
		// for this cwd (a concurrent evalMention may have started it).
		if !mentionCache.walking || mentionCache.cwd != cwd {
			mentionCache.walking = true
			mentionCache.cwd = cwd // claim this cwd so a concurrent call doesn't re-walk
			mentionCache.done = make(chan struct{})
			mentionCache.err = nil
			needWalk = true
		}
	}
	list := mentionCache.list
	done := mentionCache.done
	walking := mentionCache.walking
	mentionCache.Unlock()

	if needWalk {
		go fillMentionCache(cwd, done)
	}

	// While the first walk is in flight the cache is empty — return nothing so
	// the flyout stays open (the next keystroke re-evals against the filled
	// cache). On a large repo the walk completes well within a few keystrokes.
	var refresh tea.Cmd
	if needWalk && walking && done != nil {
		refresh = func() tea.Msg {
			<-done
			return mentionSearchMsg{cwd: cwd}
		}
	}
	if len(list) == 0 {
		return nil, refresh
	}

	lp := strings.ToLower(prefix)
	var items []mentionItem
	for _, it := range list {
		if strings.Contains(strings.ToLower(it.insert), lp) {
			items = append(items, it)
			if len(items) >= mentionMaxResults {
				break
			}
		}
	}
	sortMentionItems(items)
	return items, refresh
}

// fillMentionCache walks the CWD once and stores the result. Runs in a goroutine
// so it never blocks the UI thread; only commits if the cwd hasn't changed
// under us (a cd race), and always clears the in-progress flag.
func fillMentionCache(cwd string, done chan struct{}) {
	walked, walkErr := walkMentionListResult(cwd)
	mentionCache.Lock()
	defer mentionCache.Unlock()
	if mentionCache.cwd == cwd {
		mentionCache.list = walked
		mentionCache.at = time.Now()
		mentionCache.err = walkErr
		mentionCache.walking = false
		if mentionCache.done == done {
			mentionCache.done = nil
		}
	}
	if done != nil {
		close(done)
	}
}

// walkMentionList walks the CWD once, collecting non-ignored entries (capped at
// mentionCacheCap) with relative paths. Hidden files and heavy dirs are pruned.
func walkMentionList(cwd string) []mentionItem {
	items, _ := walkMentionListResult(cwd)
	return items
}

func walkMentionListResult(cwd string) ([]mentionItem, error) {
	if items, ok := gitMentionList(cwd); ok {
		return items, nil
	}
	var items []mentionItem
	visited := 0
	err := filepath.WalkDir(cwd, func(path string, d fs.DirEntry, err error) error {
		if path == cwd && err != nil {
			return err
		}
		if err != nil {
			return nil
		}
		visited++
		if visited > 40000 {
			return filepath.SkipAll
		}
		rel, rerr := filepath.Rel(cwd, path)
		if rerr != nil || rel == "." {
			return nil
		}
		rel = filepath.ToSlash(rel)
		name := d.Name()
		if strings.HasPrefix(name, ".") {
			if d.IsDir() {
				return filepath.SkipDir
			}
			return nil
		}
		if d.IsDir() && isIgnoredDir(name) {
			return filepath.SkipDir
		}
		isDir := d.IsDir()
		disp := rel
		ins := rel
		if isDir {
			disp += "/"
			ins += "/"
		}
		items = append(items, mentionItem{display: disp, insert: ins, isDir: isDir})
		if len(items) >= mentionCacheCap {
			return filepath.SkipAll
		}
		return nil
	})
	return items, err
}

// gitMentionList asks Git for tracked and non-ignored untracked files. Besides
// honoring the repository's real ignore policy, this avoids traversing heavy
// generated directories. Inferred directory rows preserve drill-in discovery.
func gitMentionList(cwd string) ([]mentionItem, bool) {
	cmd := exec.Command("git", "-C", cwd, "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", ".")
	out, err := cmd.Output()
	if err != nil {
		return nil, false
	}
	seen := make(map[string]bool)
	var items []mentionItem
	add := func(path string, dir bool) {
		path = filepath.ToSlash(filepath.Clean(filepath.FromSlash(path)))
		if path == "." || filepath.IsAbs(path) || strings.HasPrefix(path, "../") || seen[path] {
			return
		}
		seen[path] = true
		display, insert := path, path
		if dir {
			display += "/"
			insert += "/"
		}
		items = append(items, mentionItem{display: display, insert: insert, isDir: dir})
	}
	for _, raw := range strings.Split(string(out), "\x00") {
		path := strings.TrimPrefix(filepath.ToSlash(raw), "./")
		if path == "" {
			continue
		}
		parts := strings.Split(path, "/")
		for i := 1; i < len(parts); i++ {
			add(strings.Join(parts[:i], "/"), true)
		}
		add(path, false)
		if len(items) >= mentionCacheCap {
			break
		}
	}
	sortMentionItems(items)
	return items, true
}

func currentMentionSearchState() (mentionSearchState, error) {
	mentionCache.Lock()
	defer mentionCache.Unlock()
	if mentionCache.walking {
		return mentionSearchLoading, nil
	}
	if mentionCache.err != nil {
		return mentionSearchFailed, mentionCache.err
	}
	return mentionSearchReady, nil
}

func isIgnoredDir(name string) bool {
	switch name {
	case ".git", "node_modules", "vendor", "target", "dist", "build",
		".next", "__pycache__", ".cache", ".venv", "venv", "egg-info":
		return true
	}
	return false
}

func sortMentionItems(items []mentionItem) {
	sort.SliceStable(items, func(i, j int) bool {
		if items[i].isDir != items[j].isDir {
			return items[i].isDir // directories first
		}
		return items[i].display < items[j].display
	})
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

// renderMentionFlyout draws the flyout box shown above the input. Returns ""
// when the mention is inactive.
func (s *session) renderMentionFlyout() string {
	if !s.mentionActive {
		return ""
	}
	w := max(1, s.width)
	boxW := max(1, w-2)
	rowW := boxW - 4 // rounded border(2) + padding(2)
	rowW = max(1, rowW)

	hiStyle := lipgloss.NewStyle().
		Background(lipgloss.Color(c.dim)).
		Foreground(lipgloss.Color(c.fg)).
		Width(rowW)

	items := s.mentionItems
	n := len(items)
	if n > mentionMaxVisible {
		if s.mentionCursor < s.mentionScroll {
			s.mentionScroll = s.mentionCursor
		} else if s.mentionCursor >= s.mentionScroll+mentionMaxVisible {
			s.mentionScroll = s.mentionCursor - mentionMaxVisible + 1
		}
	} else {
		s.mentionScroll = 0
	}

	var lines []string
	state, searchErr := mentionSearchReady, error(nil)
	runes := []rune(s.input.Value())
	pos := s.input.Position()
	if s.mentionAt >= 0 && pos > s.mentionAt && pos <= len(runes) {
		query := string(runes[s.mentionAt+1 : pos])
		if query != "" && !strings.Contains(query, "/") {
			state, searchErr = currentMentionSearchState()
		}
	}
	if state == mentionSearchLoading {
		lines = append(lines, dimStyle.Render("  Searching files…"))
	} else if state == mentionSearchFailed {
		lines = append(lines, dimStyle.Render("  Search failed: "+truncateFit(searchErr.Error(), max(1, rowW-4))))
	} else if n == 0 {
		lines = append(lines, dimStyle.Render("  (no file matches)"))
	} else {
		start := s.mentionScroll
		end := start + mentionMaxVisible
		if end > n {
			end = n
		}
		for i := start; i < end; i++ {
			marker := "  "
			if i == s.mentionCursor {
				marker = accentStyle.Render("▸ ")
			}
			disp := truncateRunes(items[i].display, max(1, rowW-3))
			icon := "  "
			if items[i].isDir {
				icon = mutedStyle.Render("▾ ")
			}
			row := marker + icon + baseStyle.Render(disp)
			if i == s.mentionCursor {
				row = hiStyle.Render(row)
			}
			lines = append(lines, row)
		}
		if n > mentionMaxVisible {
			lines = append(lines, dimStyle.Render(fmt.Sprintf("  (%d more · ↑↓ scroll)", n-mentionMaxVisible)))
		}
	}
	accept, selectKey, closeKey := s.keyHint("mention_accept"), s.keyHint("select"), s.keyHint("close")
	var controls []string
	if accept != "" {
		controls = append(controls, accept+" select")
	}
	if selectKey != "" && selectKey != accept {
		controls = append(controls, selectKey+" select")
	}
	if closeKey != "" {
		controls = append(controls, closeKey+" close")
	}
	hint := dimStyle.Render("  ↑↓ navigate · " + strings.Join(controls, " · "))
	lines = append(lines, hint)
	body := strings.Join(lines, "\n")
	return lipgloss.NewStyle().
		BorderStyle(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(c.accent)).
		Padding(0, 1).
		Width(boxW).
		MaxWidth(w).
		Render(body)
}

// mentionFlyoutHeight is the screen lines the flyout claims (0 when hidden),
// so layout() can shrink the viewport to make room.
func (s *session) mentionFlyoutHeight() int {
	f := s.renderMentionFlyout()
	if f == "" {
		return 0
	}
	return lipgloss.Height(f)
}
