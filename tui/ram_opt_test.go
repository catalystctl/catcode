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
