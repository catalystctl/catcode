package main

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestCommitsMatch(t *testing.T) {
	cases := []struct {
		a, b string
		want bool
	}{
		{"abc1234", "abc1234", true},
		{"abc1234", "abc123456789", true},
		{"vabc1234", "abc1234", true},
		{"abc1234", "def5678", false},
		{"", "abc", false},
	}
	for _, c := range cases {
		if got := commitsMatch(c.a, c.b); got != c.want {
			t.Errorf("commitsMatch(%q,%q)=%v want %v", c.a, c.b, got, c.want)
		}
	}
}

func TestWebAssetName(t *testing.T) {
	if got := webAssetName("1a0228e"); got != "catcode-web-1a0228e.tar.gz" {
		t.Fatalf("got %q", got)
	}
}

func TestCoreAssetName(t *testing.T) {
	name := coreAssetName("1a0228e")
	wantPrefix := "catcode-core-1a0228e-" + osTag() + "-" + archTag()
	if name != wantPrefix+coreExeSuffix() {
		t.Fatalf("got %q want prefix %q", name, wantPrefix)
	}
}

func TestReadWebCommit(t *testing.T) {
	dir := t.TempDir()
	payload := map[string]any{"commit": "deadbee", "source": "release"}
	b, _ := json.Marshal(payload)
	if err := os.WriteFile(filepath.Join(dir, "version.json"), b, 0o644); err != nil {
		t.Fatal(err)
	}
	if got := readWebCommit(dir); got != "deadbee" {
		t.Fatalf("got %q", got)
	}
}

func TestWebDirLooksInstalled(t *testing.T) {
	dir := t.TempDir()
	if webDirLooksInstalled(dir) {
		t.Fatal("empty dir should not look installed")
	}
	if err := os.WriteFile(filepath.Join(dir, "start.js"), []byte("//"), 0o644); err != nil {
		t.Fatal(err)
	}
	if webDirLooksInstalled(dir) {
		t.Fatal("start.js alone without server/version should not count")
	}
	if err := os.WriteFile(filepath.Join(dir, "version.json"), []byte(`{"commit":"x"}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if !webDirLooksInstalled(dir) {
		t.Fatal("start.js + version.json should look installed")
	}
}

func TestExtractTarGz(t *testing.T) {
	if _, err := exec.LookPath("tar"); err != nil {
		t.Skip("tar not available")
	}
	src := t.TempDir()
	if err := os.WriteFile(filepath.Join(src, "start.js"), []byte("console.log(1)"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(src, "server.js"), []byte("//srv"), 0o644); err != nil {
		t.Fatal(err)
	}
	archive := filepath.Join(t.TempDir(), "web.tar.gz")
	cmd := exec.Command("tar", "-C", src, "-czf", archive, ".")
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("tar: %v\n%s", err, out)
	}
	dest := t.TempDir()
	if err := os.WriteFile(filepath.Join(dest, "stale.txt"), []byte("old"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := extractTarGz(archive, dest); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(dest, "stale.txt")); !os.IsNotExist(err) {
		t.Fatal("stale file should have been cleared")
	}
	got, err := os.ReadFile(filepath.Join(dest, "start.js"))
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != "console.log(1)" {
		t.Fatalf("extracted content = %q", got)
	}
}

func TestDefaultWebDirsIncludePlatformRoots(t *testing.T) {
	dirs := defaultWebDirs()
	if len(dirs) == 0 {
		t.Fatal("expected at least one default web dir")
	}
	joined := strings.Join(dirs, "|")
	switch runtime.GOOS {
	case "darwin":
		if !strings.Contains(joined, "catalyst-code") || !strings.Contains(joined, "web") {
			t.Fatalf("macOS defaults missing catalyst-code/web: %v", dirs)
		}
	case "windows":
		if !strings.Contains(joined, "catalyst-code") {
			t.Fatalf("windows defaults missing catalyst-code: %v", dirs)
		}
	default:
		if !strings.Contains(joined, "/opt/catalyst-code/web") {
			t.Fatalf("linux defaults missing /opt/catalyst-code/web: %v", dirs)
		}
	}
}

func TestDefaultWebServiceName(t *testing.T) {
	got := defaultWebServiceName()
	switch runtime.GOOS {
	case "windows":
		if got != "CatalystCodeWeb" {
			t.Fatalf("got %q", got)
		}
	case "darwin":
		if got != "com.catalyst-code.web" {
			t.Fatalf("got %q", got)
		}
	default:
		if got != "catalyst-code-web.service" {
			t.Fatalf("got %q", got)
		}
	}
}

func TestInstallerStatePathsNonEmpty(t *testing.T) {
	if len(installerStatePaths()) == 0 {
		t.Fatal("expected installer state paths")
	}
}
