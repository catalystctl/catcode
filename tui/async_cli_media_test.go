package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestMentionSearchCommandRefreshesResults(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "needle.txt"), []byte("ok"), 0o600); err != nil {
		t.Fatal(err)
	}
	old, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Chdir(dir); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Chdir(old) })

	mentionCache.Lock()
	mentionCache.cwd, mentionCache.list, mentionCache.at = "", nil, time.Time{}
	mentionCache.walking, mentionCache.done, mentionCache.err = false, nil, nil
	mentionCache.Unlock()

	s := initialSession()
	s.input.SetValue("@needle")
	s.input.SetCursor(len([]rune("@needle")))
	cmd := s.evalMention()
	if cmd == nil {
		t.Fatal("first recursive mention evaluation must return a refresh command")
	}
	if state, _ := currentMentionSearchState(); state != mentionSearchLoading {
		t.Fatalf("search state = %v, want loading", state)
	}
	msg, ok := cmd().(mentionSearchMsg)
	if !ok {
		t.Fatalf("refresh command returned %T, want mentionSearchMsg", cmd())
	}
	_ = s.handleMentionSearchMsg(msg)
	if state, err := currentMentionSearchState(); state != mentionSearchReady || err != nil {
		t.Fatalf("search state = %v, err=%v; want ready", state, err)
	}
	if len(s.mentionItems) != 1 || s.mentionItems[0].display != "needle.txt" {
		t.Fatalf("items = %#v, want needle.txt", s.mentionItems)
	}
}

func TestOwnedPendingImageMaterializesAndCleansUp(t *testing.T) {
	png := append([]byte{0x89, 'P', 'N', 'G'}, bytesOf(32)...)
	path, err := saveImageBytes(png, ".png")
	if err != nil {
		t.Fatal(err)
	}
	s := &session{pendingImages: []string{path}}
	payload := s.withImages(map[string]any{}, "")
	images, _ := payload["images"].([]string)
	if len(images) != 1 || !strings.HasPrefix(images[0], "data:image/png;base64,") {
		t.Fatalf("images = %v, want materialized PNG data URL", images)
	}
	if _, err := os.Stat(path); err != nil {
		t.Fatalf("owned temp image must remain until dispatch is accepted: %v", err)
	}
	s.clearPendingImages()
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("owned temp image still exists after accepted-dispatch cleanup: %v", err)
	}
}

func TestClearPendingImagesNeverDeletesUserFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "user.png")
	if err := os.WriteFile(path, []byte("user"), 0o600); err != nil {
		t.Fatal(err)
	}
	s := &session{pendingImages: []string{path}}
	s.clearPendingImages()
	if _, err := os.Stat(path); err != nil {
		t.Fatalf("user-owned attachment was deleted: %v", err)
	}
}

func TestHandleCLIArgsRejectsUnknownAndConflictingArgs(t *testing.T) {
	if code, handled := handleCLIArgs(nil); code != 0 || handled {
		t.Fatalf("no args = (%d,%v), want (0,false)", code, handled)
	}
	if code, handled := handleCLIArgs([]string{"--wat"}); code != 2 || !handled {
		t.Fatalf("unknown arg = (%d,%v), want (2,true)", code, handled)
	}
	if code, handled := handleCLIArgs([]string{"--version", "--help"}); code != 2 || !handled {
		t.Fatalf("conflicting args = (%d,%v), want (2,true)", code, handled)
	}
}
