package main

import (
	"strings"
	"testing"
)

// TestModalWidthResponsive: modals grow with the terminal up to a cap and
// never go below the floor (replaces the old fixed 52/60).
func TestModalWidthResponsive(t *testing.T) {
	s := initialSession()
	s.ready = true

	cases := []struct {
		termW, cap, want int
	}{
		{140, 110, 110}, // wide terminal hits the cap
		{100, 110, 96},  // terminal-4, below cap
		{60, 110, 56},   // terminal-4
		{20, 110, 20},   // compact: never wider than the terminal
		{140, 72, 72},   // settings-style cap
	}
	for _, c := range cases {
		s.width = c.termW
		if got := s.modalWidth(c.cap); got != c.want {
			t.Errorf("modalWidth(%d) at termW=%d = %d, want %d", c.cap, c.termW, got, c.want)
		}
	}
}

// TestSessionModalShowsLongTitlesWide: on a wide terminal a long session title
// is no longer truncated to the old ~20-column budget; a tail marker beyond the
// old width is visible. On a narrow terminal it is still truncated (no wrap).
func TestSessionModalShowsLongTitlesWide(t *testing.T) {
	// 70 filler chars then a marker; the marker sits well past the old fixed-52
	// modal's ~20-col label budget but inside the widened modal's budget.
	title := strings.Repeat("a", 70) + "MARKERTAIL"

	for _, tc := range []struct {
		name string
		w    int
		want bool // MARKERTAIL visible?
	}{
		{"wide", 140, true},
		{"narrow", 56, false},
	} {
		s := initialSession()
		s.ready = true
		s.width, s.height = tc.w, 24
		s.sessionList = []sessionEntry{{
			Name: "s", Path: "/tmp/s.jsonl", Title: title, Messages: 3, Mtime: 1700000000,
		}}
		s.openSessionsPicker()
		s.layout()
		box := stripANSI(s.renderModalBody())
		got := strings.Contains(box, "MARKERTAIL")
		if got != tc.want {
			t.Errorf("%s (termW=%d): MARKERTAIL visible=%v, want %v\n%s", tc.name, tc.w, got, tc.want, box)
		}
		// every visible row must be a single physical line (no wrapping)
		for _, ln := range strings.Split(box, "\n") {
			if strings.Contains(ln, "MARKERTAIL") && strings.Count(ln, "MARKERTAIL") > 1 {
				t.Errorf("%s: marker wrapped/duplicated in row: %q", tc.name, ln)
			}
		}
	}
}
