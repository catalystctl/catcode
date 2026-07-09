package main

import (
	"encoding/base64"
	"os"
	"path/filepath"
	"strings"
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

func TestExtractImagePathsQuotedAndFileURI(t *testing.T) {
	dir := t.TempDir()
	// Path with a space — only works when quoted.
	spaced := filepath.Join(dir, "my shot.png")
	if err := os.WriteFile(spaced, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	got := extractImagePaths(`see "` + spaced + `"`)
	if len(got) != 1 {
		t.Fatalf("quoted path with space: got %v", got)
	}

	// file:// URI
	plain := filepath.Join(dir, "plain.png")
	if err := os.WriteFile(plain, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	got = extractImagePaths("file://" + plain)
	if len(got) != 1 {
		t.Fatalf("file:// URI: got %v", got)
	}
}

func TestSniffImageExt(t *testing.T) {
	cases := []struct {
		name string
		b    []byte
		ext  string
	}{
		{"png", []byte{0x89, 'P', 'N', 'G', 0x0d, 0x0a, 0x1a, 0x0a}, ".png"},
		{"jpeg", []byte{0xFF, 0xD8, 0xFF, 0xE0}, ".jpg"},
		{"gif", []byte("GIF89a...."), ".gif"},
		{"webp", append([]byte("RIFF"), append([]byte{0, 0, 0, 0}, []byte("WEBP")...)...), ".webp"},
		{"bmp", []byte{0x42, 0x4D, 0, 0}, ".bmp"},
		{"not", []byte("hello world"), ""},
		{"short", []byte{0x89, 'P'}, ""},
	}
	for _, tc := range cases {
		if got := sniffImageExt(tc.b); got != tc.ext {
			t.Errorf("%s: got %q want %q", tc.name, got, tc.ext)
		}
	}
}

func TestHandlePasteContentPath(t *testing.T) {
	dir := t.TempDir()
	png := filepath.Join(dir, "clip.png")
	// Valid-looking PNG header so sniff works if someone pastes binary.
	data := append([]byte{0x89, 'P', 'N', 'G', 0x0d, 0x0a, 0x1a, 0x0a}, bytesOf(100)...)
	if err := os.WriteFile(png, data, 0o644); err != nil {
		t.Fatal(err)
	}

	s := &session{}
	res := s.handlePasteContent(png)
	if !res.consumed {
		t.Fatalf("path paste should be consumed as image, got text=%q attached=%v", res.text, res.attached)
	}
	if len(s.pendingImages) != 1 {
		t.Fatalf("expected 1 pending image, got %v", s.pendingImages)
	}
	if s.pendingImages[0] != png && filepath.Base(s.pendingImages[0]) != "clip.png" {
		// resolveImagePath returns Abs — compare bases
		if filepath.Base(s.pendingImages[0]) != "clip.png" {
			t.Fatalf("unexpected pending path %q", s.pendingImages[0])
		}
	}
}

func TestHandlePasteContentBinaryPNG(t *testing.T) {
	png := append([]byte{0x89, 'P', 'N', 'G', 0x0d, 0x0a, 0x1a, 0x0a}, bytesOf(64)...)
	s := &session{}
	res := s.handlePasteContent(string(png))
	if !res.consumed {
		t.Fatal("binary PNG paste should be consumed")
	}
	if len(s.pendingImages) != 1 {
		t.Fatalf("expected 1 pending, got %v", s.pendingImages)
	}
	// Temp file should exist and start with PNG magic.
	b, err := os.ReadFile(s.pendingImages[0])
	if err != nil {
		t.Fatal(err)
	}
	if sniffImageExt(b) != ".png" {
		t.Fatalf("saved file is not PNG")
	}
}

func TestHandlePasteContentBase64(t *testing.T) {
	png := append([]byte{0x89, 'P', 'N', 'G', 0x0d, 0x0a, 0x1a, 0x0a}, bytesOf(80)...)
	b64 := base64.StdEncoding.EncodeToString(png)
	s := &session{}
	res := s.handlePasteContent(b64)
	if !res.consumed {
		t.Fatal("base64 PNG paste should be consumed")
	}
	if len(s.pendingImages) != 1 {
		t.Fatalf("expected 1 pending, got %v", s.pendingImages)
	}
}

func TestHandlePasteContentDataURL(t *testing.T) {
	png := append([]byte{0x89, 'P', 'N', 'G', 0x0d, 0x0a, 0x1a, 0x0a}, bytesOf(80)...)
	url := "data:image/png;base64," + base64.StdEncoding.EncodeToString(png)
	s := &session{}
	res := s.handlePasteContent("look " + url + " please")
	if len(res.attached) != 1 {
		t.Fatalf("expected 1 attached, got %v", res.attached)
	}
	if !strings.Contains(res.text, "look") || !strings.Contains(res.text, "please") {
		t.Fatalf("residual text should keep surrounding words, got %q", res.text)
	}
	if res.consumed {
		t.Fatal("mixed paste should not be fully consumed")
	}
}

func TestHandlePasteContentPlainText(t *testing.T) {
	s := &session{}
	res := s.handlePasteContent("hello world\nsecond line")
	if res.consumed || len(res.attached) != 0 {
		t.Fatalf("plain text should not attach images: %+v", res)
	}
	if res.text != "hello world\nsecond line" {
		t.Fatalf("text should pass through, got %q", res.text)
	}
}

func TestWithImagesMergesPending(t *testing.T) {
	dir := t.TempDir()
	png := filepath.Join(dir, "a.png")
	if err := os.WriteFile(png, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	s := &session{pendingImages: []string{png}}
	payload := s.withImages(map[string]any{"type": "send", "prompt": "hi"}, "hi")
	imgs, _ := payload["images"].([]string)
	if len(imgs) != 1 || imgs[0] != png {
		t.Fatalf("pending should merge into images, got %v", imgs)
	}
}

func TestAddPendingImageCapAndDedup(t *testing.T) {
	s := &session{}
	if !s.addPendingImage("/tmp/a.png") {
		t.Fatal("first add should succeed")
	}
	if s.addPendingImage("/tmp/a.png") {
		t.Fatal("duplicate should be rejected")
	}
	for i := 0; i < maxPendingImages+2; i++ {
		s.addPendingImage(filepath.Join("/tmp", "img"+itoa(i)+".png"))
	}
	if len(s.pendingImages) > maxPendingImages {
		t.Fatalf("cap exceeded: %d", len(s.pendingImages))
	}
}

func itoa(i int) string {
	if i == 0 {
		return "0"
	}
	var b [16]byte
	n := len(b)
	for i > 0 {
		n--
		b[n] = byte('0' + i%10)
		i /= 10
	}
	return string(b[n:])
}

func TestPopPendingImage(t *testing.T) {
	s := &session{pendingImages: []string{"a", "b"}}
	if !s.popPendingImage() {
		t.Fatal("expected pop")
	}
	if len(s.pendingImages) != 1 || s.pendingImages[0] != "a" {
		t.Fatalf("got %v", s.pendingImages)
	}
	s.popPendingImage()
	if s.popPendingImage() {
		t.Fatal("empty pop should fail")
	}
}

func bytesOf(n int) []byte {
	b := make([]byte, n)
	for i := range b {
		b[i] = byte(i)
	}
	return b
}
