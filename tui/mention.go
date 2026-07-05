package main

import (
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
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
// in the query switches to directory-listing completion (so "@../core/sr"
// lists ../core entries beginning with "sr"); a bare query (no slash) does a
// recursive name search under the CWD. "@../" therefore reaches files outside
// the working directory, and "@/" reaches absolute paths.
// ---------------------------------------------------------------------------

const mentionMaxResults = 12
const mentionMaxVisible = 8

type mentionItem struct {
	display string // text shown in the flyout (dirs get a trailing "/")
	insert  string // path inserted after "@" on accept
	isDir   bool
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
func (s *session) evalMention() {
	s.mentionActive = false
	s.mentionItems = nil
	runes := []rune(s.input.Value())
	pos := s.input.Position()
	if pos <= 0 || pos > len(runes) {
		s.mentionCursor = 0
		s.mentionScroll = 0
		return
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
		return
	}
	// Require a word boundary before "@" so emails / "foo@bar" don't trigger.
	if at > 0 {
		prev := runes[at-1]
		if prev != ' ' && prev != '\t' && prev != '\n' {
			s.mentionCursor = 0
			s.mentionScroll = 0
			return
		}
	}
	query := string(runes[at+1 : pos])
	s.mentionAt = at
	s.mentionActive = true
	s.mentionItems = computeMentionItems(query)
	if s.mentionCursor >= len(s.mentionItems) {
		s.mentionCursor = 0
	}
	if s.mentionCursor < 0 {
		s.mentionCursor = 0
	}
	s.mentionScroll = 0
}

// handleMentionNav owns arrow/tab/enter/esc while the flyout is open. Returns
// true when it consumed the key. Printable/editing keys return false so they
// flow into the input (and then evalMention re-runs).
func (s *session) handleMentionNav(msg tea.KeyMsg) bool {
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
	var dirPart, prefix string
	if i := strings.LastIndex(query, "/"); i >= 0 {
		dirPart = query[:i]
		prefix = query[i+1:]
	} else {
		// Bare "@" or no slash: list the CWD itself.
		dirPart = "."
		prefix = query
	}
	base := dirPart
	if base == "" {
		base = "/" // query like "/foo" → list root
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
		ins := dirPart + "/" + name
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

// mentionCache memoizes the recursive file list for the CWD so a bare `@query`
// doesn't re-walk the whole tree (up to 40k entries) on every keystroke (P1-18).
// The walk is the expensive part (40k stat calls); filtering the cached list by
// prefix is microseconds. A short TTL keeps it fresh as files are added.
var mentionCache = struct {
	sync.Mutex
	cwd  string
	list []mentionItem
	at   time.Time
}{}

const mentionCacheTTL = 2 * time.Second
const mentionCacheCap = 10000

func recursiveSearch(prefix string) []mentionItem {
	cwd, err := os.Getwd()
	if err != nil {
		return nil
	}
	mentionCache.Lock()
	if mentionCache.cwd != cwd || time.Since(mentionCache.at) > mentionCacheTTL {
		mentionCache.list = walkMentionList(cwd)
		mentionCache.cwd = cwd
		mentionCache.at = time.Now()
	}
	list := mentionCache.list
	mentionCache.Unlock()

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
	return items
}

// walkMentionList walks the CWD once, collecting non-ignored entries (capped at
// mentionCacheCap) with relative paths. Hidden files and heavy dirs are pruned.
func walkMentionList(cwd string) []mentionItem {
	var items []mentionItem
	visited := 0
	_ = filepath.WalkDir(cwd, func(path string, d fs.DirEntry, err error) error {
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
	return items
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
	w := s.width
	if w < 24 {
		w = 24
	}
	boxW := w - 2
	if boxW < 20 {
		boxW = 20
	}
	rowW := boxW - 4 // rounded border(2) + padding(2)
	if rowW < 4 {
		rowW = 4
	}

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
	if n == 0 {
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
			disp := truncateRunes(items[i].display, rowW-3)
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
	hint := dimStyle.Render("  ↑↓ navigate · tab/enter select · esc close")
	lines = append(lines, hint)
	body := strings.Join(lines, "\n")
	return lipgloss.NewStyle().
		BorderStyle(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(c.accent)).
		Padding(0, 1).
		Width(boxW).
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
