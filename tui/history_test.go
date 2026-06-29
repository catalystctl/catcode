package main

import (
	"encoding/json"
	"testing"
)

func mustHistMsgs(t *testing.T, raw string) []map[string]json.RawMessage {
	t.Helper()
	var msgs []map[string]json.RawMessage
	if err := json.Unmarshal([]byte(raw), &msgs); err != nil {
		t.Fatalf("unmarshal msgs: %v", err)
	}
	return msgs
}

func TestContentText(t *testing.T) {
	cases := []struct {
		name string
		raw  string
		want string
	}{
		{"plain_string", `"hello world"`, "hello world"},
		{"null", `null`, ""},
		{"empty", `""`, ""},
		{"text_array", `[{"type":"text","text":"a"},{"type":"text","text":"b"}]`, "a\nb"},
		{"image_array", `[{"type":"text","text":"look"},{"type":"image_url","image_url":{"url":"x"}}]`, "look\n[image ×1]"},
		{"image_only", `[{"type":"image_url","image_url":{"url":"x"}}]`, "[image ×1]"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := contentText(json.RawMessage(c.raw)); got != c.want {
				t.Errorf("contentText(%s) = %q, want %q", c.raw, got, c.want)
			}
		})
	}
}

func TestRebuildBlocksFromHistory(t *testing.T) {
	s := &session{}
	msgs := mustHistMsgs(t, `[
		{"role":"user","content":"hello"},
		{"role":"assistant","reasoning_content":"let me think","content":"hi there","tool_calls":[{"id":"c1","type":"function","function":{"name":"bash","arguments":"{\"cmd\":\"ls\"}"}}]},
		{"role":"tool","tool_call_id":"c1","content":"file1\nfile2"},
		{"role":"assistant","content":"done"}
	]`)
	s.rebuildBlocksFromHistory(msgs)

	wantKinds := []blockKind{blkUser, blkThinking, blkAssistant, blkTool, blkAssistant}
	if len(s.blocks) != len(wantKinds) {
		t.Fatalf("got %d blocks, want %d", len(s.blocks), len(wantKinds))
	}
	for i, want := range wantKinds {
		if s.blocks[i].kind != want {
			t.Errorf("block %d kind = %v, want %v", i, s.blocks[i].kind, want)
		}
	}
	// User text
	if s.blocks[0].text.String() != "hello" {
		t.Errorf("user text = %q", s.blocks[0].text.String())
	}
	// Tool block: matched to its result by id, no timing.
	tb := s.blocks[3]
	if tb.name != "bash" {
		t.Errorf("tool name = %q, want bash", tb.name)
	}
	if tb.id != "c1" {
		t.Errorf("tool id = %q, want c1", tb.id)
	}
	if tb.output != "file1\nfile2" {
		t.Errorf("tool output = %q, want file1\\nfile2", tb.output)
	}
	if tb.dur == 0 {
		t.Error("tool dur = 0, want >0 (finalized)")
	}
	if !tb.started.IsZero() {
		t.Error("tool started should be zero for historical calls")
	}
	if s.cur != nil {
		t.Error("s.cur should be nil after rebuild")
	}
}

func TestRebuildBlocksFromHistoryOrphanToolResult(t *testing.T) {
	// A tool result with no matching call id becomes a standalone result block.
	s := &session{}
	msgs := mustHistMsgs(t, `[
		{"role":"user","content":"q"},
		{"role":"tool","tool_call_id":"orphan","content":"stray output"}
	]`)
	s.rebuildBlocksFromHistory(msgs)
	if len(s.blocks) != 2 {
		t.Fatalf("got %d blocks, want 2", len(s.blocks))
	}
	if s.blocks[1].kind != blkToolResult {
		t.Errorf("block 1 kind = %v, want blkToolResult", s.blocks[1].kind)
	}
	if s.blocks[1].output != "stray output" {
		t.Errorf("orphan output = %q", s.blocks[1].output)
	}
}

func TestRebuildBlocksFromHistorySkipsSystem(t *testing.T) {
	s := &session{}
	msgs := mustHistMsgs(t, `[
		{"role":"system","content":"you are a helpful agent"},
		{"role":"user","content":"hi"}
	]`)
	s.rebuildBlocksFromHistory(msgs)
	if len(s.blocks) != 1 {
		t.Fatalf("got %d blocks, want 1 (system skipped)", len(s.blocks))
	}
	if s.blocks[0].kind != blkUser {
		t.Errorf("block 0 kind = %v, want blkUser", s.blocks[0].kind)
	}
}
