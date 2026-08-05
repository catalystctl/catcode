package main

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestAppendTextSoftCap(t *testing.T) {
	b := &block{}
	b.appendText(strings.Repeat("a", maxStoredText+500))
	if b.text.Len() > maxStoredText {
		t.Fatalf("text len=%d > maxStoredText=%d", b.text.Len(), maxStoredText)
	}
	if !strings.HasSuffix(b.text.String(), storedTruncMarker) {
		t.Fatalf("missing truncation marker")
	}
	before := b.text.Len()
	b.appendText("more")
	if b.text.Len() != before {
		t.Fatalf("appended past cap: %d -> %d", before, b.text.Len())
	}
}

func TestPushHistoryCopyTrim(t *testing.T) {
	s := initialSession()
	for i := 0; i < historyMax+5; i++ {
		s.pushHistory(strings.Repeat("p", 100) + string(rune('A'+i%26)))
	}
	if len(s.history) != historyMax {
		t.Fatalf("history len=%d want %d", len(s.history), historyMax)
	}
	if cap(s.history) > historyMax+8 {
		t.Fatalf("history cap=%d; expected fresh slice near historyMax", cap(s.history))
	}
}

func TestSkillInfoOmitsContent(t *testing.T) {
	raw := []byte(`{"name":"x","description":"d","location":"L","content":"HUGE BODY"}`)
	var sk skillInfo
	if err := json.Unmarshal(raw, &sk); err != nil {
		t.Fatal(err)
	}
	if sk.Name != "x" || sk.Description != "d" {
		t.Fatalf("unexpected skillInfo: %+v", sk)
	}
	out, err := json.Marshal(sk)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(out), "content") {
		t.Fatalf("marshaled skillInfo still has content: %s", out)
	}
}

func TestCoreEventGetParsesOnce(t *testing.T) {
	raw := json.RawMessage(`{"type":"delta","text":"hi"}`)
	ev := &coreEvent{Raw: raw}
	if got := ev.get("type"); got != "delta" {
		t.Fatalf("type=%q", got)
	}
	if ev.fields == nil {
		t.Fatal("fields not cached after get")
	}
	ev.fields["text"] = json.RawMessage(`"from-fields"`)
	if got := ev.get("text"); got != "from-fields" {
		t.Fatalf("get did not use cached fields: %q", got)
	}
	v, ok := ev.rawKey("text")
	if !ok || string(v) != `"from-fields"` {
		t.Fatalf("rawKey=%s ok=%v", v, ok)
	}
}

func TestReleaseAfterCacheShrinksToolArgs(t *testing.T) {
	b := &block{
		kind:      blkTool,
		args:      strings.Repeat("a", maxStoredOutput),
		diff:      strings.Repeat("b", maxStoredOutput),
		renderStr: "cached-render",
	}
	releaseAfterCache(b)
	if b.renderStr != "" {
		t.Fatal("renderStr not cleared")
	}
	if len(b.args) > maxCachedToolArgs+len(storedTruncMarker) {
		t.Fatalf("args len=%d", len(b.args))
	}
	if len(b.diff) > maxCachedToolArgs+len(storedTruncMarker) {
		t.Fatalf("diff len=%d", len(b.diff))
	}
}

func TestRebuildHistoryReleasesMessages(t *testing.T) {
	s := initialSession()
	s.width = 80
	s.viewport.SetWidth(80)
	msgs := []map[string]json.RawMessage{
		{"role": json.RawMessage(`"user"`), "content": json.RawMessage(`"hello"`)},
		{"role": json.RawMessage(`"assistant"`), "content": json.RawMessage(`"world"`)},
	}
	s.rebuildBlocksFromHistory(msgs)
	for i, m := range msgs {
		if m != nil {
			t.Fatalf("msgs[%d] not nil'd after rebuild", i)
		}
	}
	if len(s.blocks) < 2 {
		t.Fatalf("blocks=%d", len(s.blocks))
	}
}

func TestMaxStoredOutputLowered(t *testing.T) {
	if maxStoredOutput != 64*1024 {
		t.Fatalf("maxStoredOutput=%d want 64KiB", maxStoredOutput)
	}
}

func TestTranscriptPlainIsLazy(t *testing.T) {
	s := initialSession()
	s.transcriptBase = "\x1b[31mhello\x1b[0m"
	if s.transcriptPlain != nil {
		t.Fatal("plain transcript should not be built eagerly")
	}
	lines := s.transcriptPlainLines()
	if len(lines) != 1 || lines[0] != "hello" {
		t.Fatalf("plain transcript = %#v, want [hello]", lines)
	}
	if got := s.transcriptPlainLines(); &got[0] != &lines[0] {
		t.Fatal("plain transcript should be memoized")
	}
}

func TestGlamourCacheHasByteBudget(t *testing.T) {
	if glamourBlockCacheMaxBytes <= 0 || glamourBlockCacheMaxEntryBytes <= 0 {
		t.Fatal("glamour cache byte budgets must be positive")
	}
	glamourMu.Lock()
	glamourBlockCache = map[string]string{}
	glamourCacheBytes = 0
	glamourMu.Unlock()
	t.Cleanup(func() {
		glamourMu.Lock()
		glamourBlockCache = map[string]string{}
		glamourCacheBytes = 0
		glamourMu.Unlock()
	})

	glamourMu.Lock()
	glamourBlockCache["old"] = "old"
	glamourCacheBytes = glamourBlockCacheMaxBytes - 1
	cacheGlamourBlockLocked("new", "x")
	entries, bytes := len(glamourBlockCache), glamourCacheBytes
	_, oldKept := glamourBlockCache["old"]
	_, newKept := glamourBlockCache["new"]
	glamourMu.Unlock()
	if oldKept || !newKept || entries != 1 || bytes != len("new")+len("x") {
		t.Fatalf("cache did not reset on byte budget: old=%v new=%v entries=%d bytes=%d", oldKept, newKept, entries, bytes)
	}

	glamourMu.Lock()
	cacheGlamourBlockLocked("oversized", strings.Repeat("x", glamourBlockCacheMaxEntryBytes))
	_, kept := glamourBlockCache["oversized"]
	glamourMu.Unlock()
	if kept {
		t.Fatal("oversized entry should not be retained")
	}
}
