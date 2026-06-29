package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestExtractImagePaths(t *testing.T) {
	dir := t.TempDir()
	png := filepath.Join(dir, "shot.png")
	jpg := filepath.Join(dir, "ui.jpeg")
	txt := filepath.Join(dir, "notes.txt")
	for _, p := range []string{png, jpg, txt} {
		if err := os.WriteFile(p, []byte("x"), 0o644); err != nil {
			t.Fatal(err)
		}
	}

	// Two real images mentioned (one @-prefixed, one bare); the .txt is ignored.
	got := extractImagePaths("look at @" + png + " and " + jpg + " please")
	if len(got) != 2 {
		t.Fatalf("expected 2 image paths, got %d: %v", len(got), got)
	}

	// De-duplicate the same path mentioned twice.
	got = extractImagePaths(png + " " + png)
	if len(got) != 1 {
		t.Fatalf("expected dedup to 1, got %d: %v", len(got), got)
	}

	// Non-image extension is ignored.
	if got := extractImagePaths(txt); len(got) != 0 {
		t.Fatalf(".txt should not be attached, got %v", got)
	}

	// Nonexistent image path is ignored.
	if got := extractImagePaths(filepath.Join(dir, "nope.png")); len(got) != 0 {
		t.Fatalf("nonexistent image should be ignored, got %v", got)
	}
}
