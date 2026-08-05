package main

import (
	"encoding/json"
	"strings"
	"testing"
)

// TestReasoningSurvivesTurnCompletion covers the stream-cache boundary. A
// short final thinking delta can be below streamBatch, so it may not have been
// rendered while the block is live. Finalizing the block (when a tool call or
// done event arrives) must render the complete reasoning text rather than
// reusing that stale live snapshot.
func TestReasoningSurvivesTurnCompletion(t *testing.T) {
	run := func(name string, terminalEvents ...string) {
		t.Run(name, func(t *testing.T) {
			s := initialSession()
			s.ready = true
			s.authed = true
			s.width = 100
			s.height = 30
			s.models = []modelInfo{{ID: "test-model"}}
			s.modelIdx = 0
			// The default UI collapses reasoning. Expand it here so this test
			// verifies the actual reasoning body remains visible after the turn
			// is finalized.
			s.thinkExpanded = true
			s.settings.ThinkExpanded = true
			s.layout()

			makeEvent := func(raw string) *coreEvent {
				e := &coreEvent{Raw: json.RawMessage(raw)}
				var fields map[string]json.RawMessage
				if err := json.Unmarshal(e.Raw, &fields); err != nil {
					t.Fatalf("parse event %s: %v", raw, err)
				}
				e.fields = fields
				if err := json.Unmarshal(fields["type"], &e.Type); err != nil {
					t.Fatalf("parse event type %s: %v", raw, err)
				}
				return e
			}

			// Render the first snapshot, then append a short final delta. The
			// latter is intentionally below streamBatch and therefore remains
			// absent from the cached live render until the block is finalized.
			s.handleCoreEvent(makeEvent(`{"type":"thinking","text":"initial reasoning"}`))
			s.refresh()
			s.handleCoreEvent(makeEvent(`{"type":"thinking","text":" and the final conclusion"}`))

			for _, raw := range terminalEvents {
				s.handleCoreEvent(makeEvent(raw))
			}

			seen := stripANSI(s.viewport.View())
			if !strings.Contains(seen, "initial reasoning") || !strings.Contains(seen, "final conclusion") {
				t.Fatalf("finalized transcript lost reasoning:\n%s", seen)
			}
		})
	}

	// The direct done path exercises finalized-cache reuse without another push.
	run("done", `{"type":"done"}`)
	// The tool path exercises push(), which moves the live reasoning block into
	// the immutable prefix before rendering the activity group.
	run("tool transition",
		`{"type":"tool_call","name":"read_file","id":"call-1","args":"{\"path\":\"a\"}"}`,
		`{"type":"tool_result","id":"call-1","ok":"true","output":"contents"}`,
		`{"type":"done"}`,
	)
}
