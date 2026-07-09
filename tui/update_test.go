package main

import (
	"testing"
)

func TestCompareUpdate(t *testing.T) {
	saved := coreVersion
	t.Cleanup(func() { coreVersion = saved })

	cases := []struct {
		name    string
		cur     string
		latest  string
		wantNil bool
		wantCur string
		wantLat string
	}{
		{"up to date", "abc1234", "abc1234", true, "", ""},
		{"update available", "abc1234", "def5678", false, "abc1234", "def5678"},
		{"dev build never nags", "dev", "def5678", true, "", ""},
		{"empty latest", "abc1234", "", true, "", ""},
		{"dev == dev", "dev", "dev", true, "", ""},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			coreVersion = c.cur
			got := compareUpdate(c.latest)
			if c.wantNil {
				if got != nil {
					t.Fatalf("expected nil, got %+v", got)
				}
				return
			}
			if got == nil {
				t.Fatalf("expected non-nil updateInfo")
			}
			if got.current != c.wantCur || got.latest != c.wantLat {
				t.Fatalf("got {current:%s latest:%s}, want {%s %s}", got.current, got.latest, c.wantCur, c.wantLat)
			}
		})
	}
}

func TestAssetName(t *testing.T) {
	// Deterministic given the version + runtime platform. Verifies the suffix
	// mapping (amd64->x86_64, darwin->macos) and the .exe suffix on Windows.
	name := assetName("1a0228e")
	switch osTag() {
	case "linux":
		switch archTag() {
		case "x86_64":
			if name != "catcode-1a0228e-linux-x86_64" {
				t.Fatalf("got %q", name)
			}
		case "arm64":
			if name != "catcode-1a0228e-linux-arm64" {
				t.Fatalf("got %q", name)
			}
		}
	case "macos":
		switch archTag() {
		case "x86_64":
			if name != "catcode-1a0228e-macos-x86_64" {
				t.Fatalf("got %q", name)
			}
		case "arm64":
			if name != "catcode-1a0228e-macos-arm64" {
				t.Fatalf("got %q", name)
			}
		}
	case "windows":
		if name != "catcode-1a0228e-windows-x86_64.exe" {
			t.Fatalf("got %q", name)
		}
	}
}

func TestHumanBytes(t *testing.T) {
	cases := []struct{ in int64; want string }{
		{512, "512B"},
		{2048, "2.0KB"},
		{1 << 20, "1.0MB"},
		{1 << 30, "1.0GB"},
	}
	for _, c := range cases {
		if got := humanBytes(c.in); got != c.want {
			t.Errorf("humanBytes(%d) = %q, want %q", c.in, got, c.want)
		}
	}
}

func TestFindAsset(t *testing.T) {
	rel := &ghRelease{TagName: "x", Assets: []ghAsset{
		{Name: "catcode-x-linux-x86_64", BrowserDownloadURL: "u1"},
		{Name: "catcode-x-linux-arm64", BrowserDownloadURL: "u2"},
		{Name: "other", BrowserDownloadURL: "u3"},
	}}
	if a := findAsset(rel, "catcode-x-linux-arm64"); a == nil || a.BrowserDownloadURL != "u2" {
		t.Fatal("expected to find the arm64 asset")
	}
	if a := findAsset(rel, "missing"); a != nil {
		t.Fatalf("expected nil for missing asset, got %+v", a)
	}
}
