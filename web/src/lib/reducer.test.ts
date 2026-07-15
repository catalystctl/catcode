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

describe("metrics tps_est", () => {
  test("maps mid-stream tps_est onto tps", () => {
    const s = reduce(initialState, {
      type: "metrics",
      tps_est: 42.5,
    } as AgentEvent);
    expect(s.metrics?.tps).toBe(42.5);
    expect(s.retrying).toBe(false);
  });

  test("prefers final tps over tps_est", () => {
    let s = reduce(initialState, { type: "metrics", tps_est: 10 } as AgentEvent);
    s = reduce(s, { type: "metrics", tps: 55, elapsed_ms: 1000 } as AgentEvent);
    expect(s.metrics?.tps).toBe(55);
  });
});

describe("intercom need_decision", () => {
  test("empty reason is log-only (not a blocking ask)", () => {
    const s = reduce(initialState, {
      type: "intercom_message",
      id: "m1",
      from: "explorer",
      to: "orchestrator",
      reason: "",
      message: "progress note",
    } as AgentEvent);
    expect(s.pendingIntercom).toBeNull();
    expect(s.intercomLog[0]?.kind).toBe("reply");
  });

  test("need_decision opens pendingIntercom", () => {
    const s = reduce(initialState, {
      type: "intercom_message",
      id: "m2",
      from: "explorer",
      to: "orchestrator",
      reason: "need_decision",
      message: "which path?",
    } as AgentEvent);
    expect(s.pendingIntercom?.request_id).toBe("m2");
    expect(s.pendingIntercom?.message).toBe("which path?");
  });
});

describe("config_changed", () => {
  test("patches ready.bash_timeout_secs and auto_compact", () => {
    let s = reduce(initialState, {
      type: "ready",
      models: [],
      authed: true,
      workspace: "/tmp",
      approval: "destructive",
      base_url: "",
      provider: "x",
      providerKind: "openai",
      providers: ["x"],
      bash_timeout_secs: 30,
      auto_compact: true,
      resumed_messages: 0,
    } as AgentEvent);
    s = reduce(s, { type: "config_changed", key: "bash_timeout_secs", value: 90 });
    expect(s.ready?.bash_timeout_secs).toBe(90);
    s = reduce(s, { type: "config_changed", key: "auto_compact", value: false });
    expect(s.ready?.auto_compact).toBe(false);
  });

  test("patches ready.sandbox", () => {
    let s = reduce(initialState, {
      type: "ready",
      models: [],
      authed: true,
      workspace: "/tmp",
      approval: "destructive",
      base_url: "",
      provider: "x",
      providerKind: "openai",
      providers: ["x"],
      bash_timeout_secs: 30,
      sandbox: "none",
      resumed_messages: 0,
    } as AgentEvent);
    s = reduce(s, { type: "config_changed", key: "sandbox", value: "firejail" });
    expect(s.ready?.sandbox).toBe("firejail");
  });
});

describe("diagnostics + agents", () => {
  test("stores context breakdown and agents list", () => {
    let s = reduce(initialState, {
      type: "context_breakdown",
      total_tokens: 1000,
      context_window: 8000,
      pct: 12,
      messages: 4,
      system_tokens: 200,
      by_role: { user: 300 },
      top_consumers: [{ index: 1, role: "user", tokens: 300, preview: "hi" }],
    } as AgentEvent);
    expect(s.contextBreakdown?.total_tokens).toBe(1000);
    expect(s.toasts).toHaveLength(0);
    s = reduce(s, {
      type: "agents",
      agents: [{ name: "scout", description: "explore", source: "builtin" }],
    } as AgentEvent);
    expect(s.availableAgents).toEqual([
      { name: "scout", description: "explore", source: "builtin" },
    ]);
  });
});

describe("http_retry clears on delta", () => {
  test("retrying flag clears when streaming resumes", () => {
    let s = reduce(initialState, { type: "http_retry", status: 429 } as AgentEvent);
    expect(s.retrying).toBe(true);
    s = reduce(s, { type: "delta", text: "hi" });
    expect(s.retrying).toBe(false);
  });
});

describe("finishTurn clears gates", () => {
  test("aborted drops pending approval/ask/sudo/intercom and follow-up queue", () => {
    let s = reduce(initialState, {
      type: "approval_request",
      request_id: "a1",
      tool: "bash",
      args: "{}",
    } as AgentEvent);
    s = reduce(s, {
      type: "info",
      message: "prompt queued; will run after the current turn",
    } as AgentEvent);
    expect(s.followUpQueued).toBe(true);
    expect(s.pendingApproval).not.toBeNull();
    s = reduce(s, { type: "aborted" });
    expect(s.pendingApproval).toBeNull();
    expect(s.followUpQueued).toBe(false);
    expect(s.streaming).toBe(false);
  });
});

describe("error does not end the turn", () => {
  // Covered by "pre-turn error clears streaming" — kept as a pointer in history.
});

describe("pre-turn error clears streaming", () => {
  test("error after optimistic user with no assistant clears streaming", () => {
    let s = reduce(initialState, { type: "_user", text: "/skill:missing" });
    expect(s.streaming).toBe(true);
    s = reduce(s, { type: "error", message: "unknown skill" });
    expect(s.streaming).toBe(false);
  });

  test("error mid-turn keeps streaming", () => {
    let s = reduce(initialState, { type: "_user", text: "hi" });
    s = reduce(s, { type: "delta", text: "hello" });
    expect(s.streaming).toBe(true);
    s = reduce(s, { type: "error", message: "no pending approval" });
    expect(s.streaming).toBe(true);
  });
});

describe("digested without before_tokens", () => {
  test("subagent digested does not throw", () => {
    const s = reduce(initialState, {
      type: "digested",
      results: 2,
      after_tokens: 1000,
    } as AgentEvent);
    expect(s.toasts[0].message).toContain("Reclaimed");
    expect(s.toasts[0].message).toContain("1,000");
  });
});

describe("pre-turn error drops ghost user", () => {
  test("unknown skill error removes optimistic user line", () => {
    let s = reduce(initialState, { type: "_user", text: "/skill:missing" });
    expect(s.messages).toHaveLength(1);
    s = reduce(s, { type: "error", message: "unknown skill" });
    expect(s.messages).toHaveLength(0);
    expect(s.streaming).toBe(false);
  });

  test("core exited error keeps the last user line", () => {
    let s = reduce(initialState, { type: "_user", text: "hi" });
    s = reduce(s, { type: "aborted" });
    expect(s.messages).toHaveLength(1);
    s = reduce(s, {
      type: "error",
      message: "This session's core exited. Sending a message will restart it.",
    });
    expect(s.messages).toHaveLength(1);
    expect(s.goalMode).toBeNull();
  });
});

describe("models rebinds selectedModel", () => {
  test("clears selection when model disappears from list", () => {
    let s = reduce(initialState, {
      type: "models",
      models: [
        { id: "a", name: "A", provider: "p" },
        { id: "b", name: "B", provider: "p" },
      ],
    } as never);
    s = reduce(s, { type: "_select_model", id: "b" });
    expect(s.selectedModel).toBe("b");
    s = reduce(s, {
      type: "models",
      models: [{ id: "a", name: "A", provider: "p" }],
    } as never);
    expect(s.selectedModel).toBe("a");
  });
});

describe("history tokens_in", () => {
  test("history with tokens_in seeds stats", () => {
    const s = reduce(initialState, {
      type: "history",
      messages: [{ role: "user", content: "hi" }],
      tokens_in: 42,
    } as never);
    expect(s.stats?.tokens_in).toBe(42);
    expect(s.messages).toHaveLength(1);
  });
});

describe("goal_state idle clears plan", () => {
  test("idle clears goalMode and goalPlan", () => {
    const seeded = {
      ...initialState,
      goalMode: {
        id: "g1",
        goal: "x",
        phase: "running",
        concurrency: 1,
        max_tasks: 1,
        allowed_models: [] as string[],
        allowed_providers: [] as string[],
        auto_deploy: false,
        prompts: [],
        active_run_ids: [],
        version: 1,
        error: null,
        parent_model: "",
      },
      goalPlan: {
        id: "g1",
        summary: "plan",
        steps: [],
        risks: [],
        validation: [],
        version: 1,
      },
    };
    const s = reduce(seeded, { type: "goal_state", id: "", phase: "idle" } as never);
    expect(s.goalMode).toBeNull();
    expect(s.goalPlan).toBeNull();
  });
});

describe("goal deployment streaming lifecycle", () => {
  const goalState = (phase: string, autoDeploy = true) =>
    ({
      type: "goal_state",
      id: "g1",
      goal: "ship it",
      phase,
      concurrency: 2,
      max_tasks: 4,
      allowed_models: [],
      allowed_providers: [],
      auto_deploy: autoDeploy,
      prompts: [],
      active_run_ids: [],
      version: 1,
      error: null,
      parent_model: "model",
    }) as AgentEvent;

  test("planning done stays live while an auto-deploy plan is handed off", () => {
    let s = reduce(initialState, { type: "_user", text: "ship it" });
    s = reduce(s, goalState("plan_ready", true));
    s = reduce(s, { type: "done" });
    expect(s.streaming).toBe(true);
  });

  test("deploying re-arms streaming even when it arrives after planning done", () => {
    let s = reduce(initialState, { type: "done" });
    expect(s.streaming).toBe(false);
    s = reduce(s, goalState("deploying"));
    expect(s.streaming).toBe(true);
    s = reduce(s, goalState("running"));
    expect(s.streaming).toBe(true);
  });

  test("manual plan review and terminal goal phases are idle", () => {
    let s = reduce(initialState, { type: "_user", text: "ship it" });
    s = reduce(s, goalState("plan_ready", false));
    s = reduce(s, { type: "done" });
    expect(s.streaming).toBe(false);

    s = reduce(s, goalState("deploying"));
    expect(s.streaming).toBe(true);
    s = reduce(s, goalState("synthesizing"));
    expect(s.streaming).toBe(true);
    s = reduce(s, { type: "done" });
    expect(s.streaming).toBe(true);
    s = reduce(s, goalState("done"));
    expect(s.streaming).toBe(false);
  });
});

describe("undo local + pendingUndo reset", () => {
  test("_undo_local trims last turn and sets pendingUndo", () => {
    let s = reduce(initialState, { type: "_user", text: "one" });
    s = reduce(s, { type: "delta", text: "a1" });
    s = reduce(s, { type: "done" });
    s = reduce(s, { type: "_user", text: "two" });
    s = reduce(s, { type: "delta", text: "a2" });
    s = reduce(s, { type: "done" });
    expect(s.messages.length).toBeGreaterThanOrEqual(4);
    s = reduce(s, { type: "_undo_local" });
    expect(s.pendingUndo).toBe(true);
    expect(s.messages.some((m) => m.role === "user" && (m as { text: string }).text === "two")).toBe(false);
    expect(s.messages.some((m) => m.role === "user" && (m as { text: string }).text === "one")).toBe(true);
    s = reduce(s, { type: "reset" });
    expect(s.pendingUndo).toBe(false);
    expect(s.messages.some((m) => m.role === "user" && (m as { text: string }).text === "one")).toBe(true);
  });

  test("plain reset still clears messages", () => {
    let s = reduce(initialState, { type: "_user", text: "x" });
    s = reduce(s, { type: "reset" });
    expect(s.messages).toHaveLength(0);
  });

  test("_undo_local is idempotent (client + fanout)", () => {
    let s = reduce(initialState, { type: "_user", text: "one" });
    s = reduce(s, { type: "delta", text: "a1" });
    s = reduce(s, { type: "done" });
    s = reduce(s, { type: "_user", text: "two" });
    s = reduce(s, { type: "delta", text: "a2" });
    s = reduce(s, { type: "done" });
    const afterFirst = reduce(s, { type: "_undo_local" });
    const afterSecond = reduce(afterFirst, { type: "_undo_local" });
    expect(afterSecond.messages.length).toBe(afterFirst.messages.length);
    expect(afterSecond.pendingUndo).toBe(true);
  });
});

describe("core protocol events", () => {
  test("cost_update stores session cost", () => {
    const s = ev({
      type: "cost_update",
      tokens_in: 100,
      tokens_out: 40,
      estimated_usd: 0.0123,
      model: "glm-5.2",
    });
    expect(s.cost?.estimated_usd).toBe(0.0123);
    expect(s.cost?.tokens_in).toBe(100);
  });

  test("file_change bumps seq and records path", () => {
    const s = ev({
      type: "file_change",
      path: "src/a.ts",
      tool: "edit",
    });
    expect(s.fileChangeSeq).toBe(1);
    expect(s.recentFileChanges[0]?.path).toBe("src/a.ts");
  });

  test("checkpoint_created + checkpoints list", () => {
    let s = ev({
      type: "checkpoint_created",
      id: "cp1",
      label: "manual",
      kind: "file",
      auto: false,
    });
    expect(s.checkpoints[0]?.id).toBe("cp1");
    expect(s.toasts.some((t) => t.message.includes("Checkpoint"))).toBe(true);
    s = reduce(s, {
      type: "checkpoints",
      checkpoints: [{ id: "cp2", label: "other", kind: "git" }],
    });
    expect(s.checkpoints).toHaveLength(1);
    expect(s.checkpoints[0].id).toBe("cp2");
  });

  test("protocol_hello stores capabilities", () => {
    const s = ev({
      type: "protocol_hello",
      version: "1",
      min_client: "1",
      capabilities: ["checkpoints", "worktrees"],
    });
    expect(s.protocolHello?.capabilities).toContain("checkpoints");
  });

  test("goal_step_verdict toasts and stores result", () => {
    const s = ev({ type: "goal_step_verdict", ok: false, output: "tests failed" });
    expect(s.lastGoalVerdict?.ok).toBe(false);
    expect(s.toasts.some((t) => t.kind === "error")).toBe(true);
  });

  test("session_pinned updates session list", () => {
    let s = reduce(initialState, {
      type: "sessions",
      sessions: [{ name: "a.jsonl", mtime: 1, path: "/tmp/a.jsonl" }],
      files: [],
    });
    s = reduce(s, { type: "session_pinned", path: "/tmp/a.jsonl", pinned: true });
    expect(s.sessions[0].pinned).toBe(true);
  });
});

describe("CORE_EVENT_TYPES coverage", () => {
  test("reducer has an explicit case for every SDK core event type", async () => {
    const { CORE_EVENT_TYPES } = await import("@catalyst-code/coding-agent");
    const src = await Bun.file(new URL("./reducer.ts", import.meta.url)).text();
    const cases = new Set(
      [...src.matchAll(/case\s+"([a-z0-9_]+)":/g)].map((m) => m[1]),
    );
    // Every event the SDK catalog knows about must be handled.
    const missing = CORE_EVENT_TYPES.filter((t) => !cases.has(t));
    expect(missing).toEqual([]);
  });

  test("every reducer-handled core event is in the SDK catalog (or documented as a gap)", async () => {
    const { CORE_EVENT_TYPES } = await import("@catalyst-code/coding-agent");
    const sdkSet: Set<string> = new Set(CORE_EVENT_TYPES);
    const src = await Bun.file(new URL("./reducer.ts", import.meta.url)).text();
    const allCases = [...src.matchAll(/case\s+"([a-z0-9_]+)":/g)].map((m) => m[1]);
    // Synthetic events (underscore-prefixed) are not core wire events.
    const coreEvents = allCases.filter((c) => !c.startsWith("_"));
    // Bridge-generated events (not core wire events); correctly absent from CORE_EVENT_TYPES.
    const bridgeEvents = new Set(["projects", "workspace_changed"]);
    const extra = coreEvents.filter((c) => !sdkSet.has(c) && !bridgeEvents.has(c));
    expect(extra).toEqual([]);
  });
});
