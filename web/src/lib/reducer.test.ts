// Reducer unit tests — exercises the pure reduce() function for the key
// message-assembly rules and state transitions. Run with `bun test`.
import { test, expect, describe } from "bun:test";
import { reduce, initialState, MAX_TERMINAL_RUNS } from "./reducer";
import type { AgentEvent, UIMessage } from "./types";

const ev = (e: AgentEvent) => reduce(initialState, e);

// Type guards so we can access role-specific fields without casts.
function isUser(m: UIMessage): m is import("./types").UserMsg {
  return m.role === "user";
}
function isAssistant(m: UIMessage): m is import("./types").AssistantMsg {
  return m.role === "assistant";
}

describe("initial state", () => {
  test("has sensible defaults", () => {
    expect(initialState.ready).toBeNull();
    expect(initialState.messages).toEqual([]);
    expect(initialState.streaming).toBe(false);
    expect(initialState.switching).toBe(false);
    expect(initialState.projects).toEqual([]);
  });
});

describe("synthetic events", () => {
  test("_user adds a user message and sets streaming", () => {
    const s = ev({ type: "_user", text: "hello" });
    expect(s.messages).toHaveLength(1);
    expect(s.messages[0].role).toBe("user");
    expect(isUser(s.messages[0]) ? s.messages[0].text : "").toBe("hello");
    expect(s.streaming).toBe(true);
  });

  test("_user with steer flag marks the message", () => {
    const s = ev({ type: "_user", text: "redirect", steer: true });
    expect(s.messages[0].role).toBe("user");
    expect(isUser(s.messages[0]) ? s.messages[0].steer : false).toBe(true);
  });

  test("_select_model sets the selected model", () => {
    const s = ev({ type: "_select_model", id: "glm-5.2" });
    expect(s.selectedModel).toBe("glm-5.2");
  });

  test("_set_switching toggles the switching flag", () => {
    const s = ev({ type: "_set_switching", switching: true });
    expect(s.switching).toBe(true);
    const s2 = reduce(s, { type: "_set_switching", switching: false });
    expect(s2.switching).toBe(false);
  });

  test("_dismiss_toast removes a toast by id", () => {
    let s = ev({ type: "error", message: "boom" });
    const id = s.toasts[0].id;
    s = reduce(s, { type: "_dismiss_toast", id });
    expect(s.toasts).toHaveLength(0);
  });
});

describe("assistant message assembly", () => {
  test("delta lazily begins an assistant message", () => {
    const s = ev({ type: "delta", text: "Hi" });
    expect(s.messages).toHaveLength(1);
    expect(s.messages[0].role).toBe("assistant");
    expect(isAssistant(s.messages[0]) ? s.messages[0].text : "").toBe("Hi");
    expect(s.currentAssistantId).not.toBeNull();
    expect(s.streaming).toBe(true);
  });

  test("multiple deltas append to the same assistant message", () => {
    let s = ev({ type: "delta", text: "Hello" });
    s = reduce(s, { type: "delta", text: " world" });
    expect(s.messages).toHaveLength(1);
    expect(isAssistant(s.messages[0]) ? s.messages[0].text : "").toBe("Hello world");
  });

  test("done finalizes the assistant and clears streaming", () => {
    let s = ev({ type: "delta", text: "Hi" });
    expect(s.streaming).toBe(true);
    s = reduce(s, { type: "done" });
    expect(s.streaming).toBe(false);
    expect(s.currentAssistantId).toBeNull();
    expect(isAssistant(s.messages[0]) ? s.messages[0].streaming : false).toBe(false);
  });

  test("thinking accumulates into the assistant message", () => {
    let s = ev({ type: "thinking", text: "hmm" });
    s = reduce(s, { type: "thinking", text: "..." });
    expect(isAssistant(s.messages[0]) ? s.messages[0].thinking : "").toBe("hmm...");
  });
});

describe("tool calls", () => {
  test("tool_call finalizes the current assistant and attaches the call", () => {
    let s = ev({ type: "delta", text: "Let me check" });
    s = reduce(s, { type: "tool_call", id: "t1", name: "read_file", args: '{"path":"a.ts"}' });
    const a = s.messages[0];
    expect(a.role).toBe("assistant");
    if (!isAssistant(a)) throw new Error("expected assistant");
    expect(a.toolCalls).toHaveLength(1);
    expect(a.toolCalls[0].name).toBe("read_file");
    expect(a.toolCalls[0].args.path).toBe("a.ts");
  });

  test("tool_result matches its tool call by id", () => {
    let s = ev({ type: "delta", text: "" });
    s = reduce(s, { type: "tool_call", id: "t1", name: "bash", args: "{}" });
    s = reduce(s, { type: "tool_result", id: "t1", ok: true, output: "done" });
    const a = s.messages[0];
    if (!isAssistant(a)) throw new Error("expected assistant");
    expect(a.toolCalls[0].result?.ok).toBe(true);
    expect(a.toolCalls[0].result?.output).toBe("done");
  });

  test("a delta after tool_call begins a fresh assistant message", () => {
    let s = ev({ type: "delta", text: "first" });
    s = reduce(s, { type: "tool_call", id: "t1", name: "bash", args: "{}" });
    s = reduce(s, { type: "tool_result", id: "t1", ok: true, output: "ok" });
    s = reduce(s, { type: "delta", text: "second" });
    expect(s.messages).toHaveLength(2);
    expect(s.messages[1].role).toBe("assistant");
    expect(isAssistant(s.messages[1]) ? s.messages[1].text : "").toBe("second");
  });
});

describe("sessions", () => {
  test("sorted by mtime descending", () => {
    const s = ev({
      type: "sessions",
      sessions: [
        { name: "old.jsonl", mtime: 100 },
        { name: "new.jsonl", mtime: 200 },
      ],
      files: [],
    });
    expect(s.sessions[0].name).toBe("new.jsonl");
    expect(s.sessions[1].name).toBe("old.jsonl");
  });

  test("preserves title field", () => {
    const s = ev({
      type: "sessions",
      sessions: [{ name: "a.jsonl", mtime: 1, title: "My Session", messages: 5 }],
      files: [],
    });
    expect(s.sessions[0].title).toBe("My Session");
    expect(s.sessions[0].messages).toBe(5);
  });
});

describe("session rename overlay", () => {
  test("_session_title updates a session's title", () => {
    let s = ev({
      type: "sessions",
      sessions: [{ name: "a.jsonl", mtime: 1, title: "old" }],
      files: [],
    });
    s = reduce(s, { type: "_session_title", name: "a.jsonl", title: "New Name" });
    expect(s.sessions[0].title).toBe("New Name");
  });

  test("_session_title with empty title clears it", () => {
    let s = ev({
      type: "sessions",
      sessions: [{ name: "a.jsonl", mtime: 1, title: "old" }],
      files: [],
    });
    s = reduce(s, { type: "_session_title", name: "a.jsonl", title: "" });
    expect(s.sessions[0].title).toBeUndefined();
  });
});

describe("workspace switching", () => {
  test("workspace_changed resets per-session state and clears switching", () => {
    let s = ev({ type: "_user", text: "hi" });
    s = reduce(s, { type: "_set_switching", switching: true });
    s = reduce(s, { type: "workspace_changed", workspace: "/new/path", projects: [] });
    expect(s.workspace).toBe("/new/path");
    expect(s.switching).toBe(false);
    expect(s.messages).toEqual([]);
    expect(s.currentSessionFile).toBeNull();
    expect(s.stats).toBeNull();
  });

  test("projects event sets the project list", () => {
    const s = ev({
      type: "projects",
      projects: [{ path: "/a", name: "a", lastUsed: 1 }],
    });
    expect(s.projects).toHaveLength(1);
    expect(s.projects[0].name).toBe("a");
  });
});

describe("toasts", () => {
  test("error pushes an error toast", () => {
    const s = ev({ type: "error", message: "failed" });
    expect(s.toasts).toHaveLength(1);
    expect(s.toasts[0].kind).toBe("error");
    expect(s.toasts[0].message).toBe("failed");
  });

  test("compacted pushes an info toast", () => {
    const s = ev({ type: "compacted", before_tokens: 1000, after_tokens: 500 });
    expect(s.toasts[0].kind).toBe("info");
    expect(s.toasts[0].message).toContain("1,000");
  });

  test("toasts are capped at 6", () => {
    let s = initialState;
    for (let i = 0; i < 10; i++) s = reduce(s, { type: "info", message: `msg ${i}` });
    expect(s.toasts.length).toBeLessThanOrEqual(6);
  });
});

describe("reset", () => {
  test("clears messages and streaming", () => {
    let s = ev({ type: "_user", text: "hi" });
    s = reduce(s, { type: "delta", text: "hey" });
    s = reduce(s, { type: "reset" });
    expect(s.messages).toEqual([]);
    expect(s.streaming).toBe(false);
    expect(s.currentAssistantId).toBeNull();
  });
});

describe("history replay", () => {
  test("extracts images from multimodal user messages", () => {
    const s = ev({
      type: "history",
      messages: [
        {
          role: "user",
          content: [
            { type: "text", text: "What is this?" },
            { type: "image_url", image_url: { url: "data:image/png;base64,abc" } },
          ],
        },
      ],
    });
    expect(s.messages).toHaveLength(1);
    const u = s.messages[0];
    expect(u.role).toBe("user");
    if (!isUser(u)) throw new Error("expected user");
    expect(u.text).toBe("What is this?");
    expect(u.images).toEqual(["data:image/png;base64,abc"]);
  });

  test("marks replayed tool results as unknown (no live ok/error)", () => {
    const s = ev({
      type: "history",
      messages: [
        { role: "user", content: "list files" },
        {
          role: "assistant",
          content: null,
          tool_calls: [{ id: "t1", type: "function", function: { name: "list_dir", arguments: "{}" } }],
        },
        { role: "tool", tool_call_id: "t1", content: "a\nb\nc" },
      ],
    });
    const a = s.messages.find((m) => m.role === "assistant");
    if (!a || !isAssistant(a)) throw new Error("expected assistant");
    expect(a.toolCalls).toHaveLength(1);
    expect(a.toolCalls[0].result?.output).toBe("a\nb\nc");
    expect(a.toolCalls[0].result?.unknown).toBe(true);
  });
});

describe("reset clears stale prompts", () => {
  test("reset drops a pending intercom ask", () => {
    let s = ev({
      type: "intercom_message",
      id: "ic1",
      from: "worker",
      message: "decide?",
      reason: "need_decision",
    });
    expect(s.pendingIntercom).not.toBeNull();
    s = reduce(s, { type: "reset" });
    expect(s.pendingIntercom).toBeNull();
  });
});

describe("subagent run pruning", () => {
  test("terminal runs are pruned past the cap (no unbounded growth)", () => {
    let s = initialState;
    const N = MAX_TERMINAL_RUNS + 40;
    for (let i = 0; i < N; i++) {
      const id = `run-${i}`;
      s = reduce(s, {
        type: "subagent_start",
        run_id: id,
        mode: "single",
        agent: "worker",
        agents: ["worker"],
        task: "t",
        depth: 0,
        started_at: 1000 + i,
      });
      s = reduce(s, {
        type: "subagent_done",
        run_id: id,
        state: "completed",
        ended_at: 2000 + i,
      });
    }
    expect(Object.keys(s.subagentRuns).length).toBeLessThanOrEqual(MAX_TERMINAL_RUNS);
    // Most-recent terminal run is retained; oldest was pruned.
    expect(s.subagentRuns[`run-${N - 1}`]).toBeDefined();
    expect(s.subagentRuns["run-0"]).toBeUndefined();
  });

  test("running runs are never pruned even past the cap", () => {
    let s = initialState;
    const N = MAX_TERMINAL_RUNS + 12;
    for (let i = 0; i < N; i++) {
      const id = `run-${i}`;
      s = reduce(s, {
        type: "subagent_start",
        run_id: id,
        mode: "single",
        agents: [],
        task: "t",
        depth: 0,
        started_at: i,
      });
      // leave odd-indexed runs running; complete even ones.
      if (i % 2 === 0)
        s = reduce(s, { type: "subagent_done", run_id: id, state: "completed", ended_at: i });
    }
    for (let i = 1; i < N; i += 2) {
      expect(s.subagentRuns[`run-${i}`]).toBeDefined();
      expect(s.subagentRuns[`run-${i}`].state).toBe("running");
    }
  });
});
