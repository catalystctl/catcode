import { afterAll, describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// hub-layout-store reads homedir() — point HOME at a temp dir for isolation.
const tmp = mkdtempSync(join(tmpdir(), "hub-layout-"));
const prevHome = process.env.HOME;
process.env.HOME = tmp;

const {
  defaultHubState,
  loadHubLayout,
  sanitizeHubState,
  saveHubLayout,
} = await import("../server/hub-layout-store");

afterAll(() => {
  process.env.HOME = prevHome;
  try {
    rmSync(tmp, { recursive: true, force: true });
  } catch {
    /* ignore */
  }
});

describe("hub layout store (chat hub v2)", () => {
  test("default state is empty", () => {
    const s = defaultHubState();
    expect(s.version).toBe(2);
    expect(s.tabPaths).toEqual([]);
    expect(s.active).toBeNull();
    expect(s.sessions).toEqual({});
    expect(s.gitOpen).toBe(true);
  });

  test("sanitize migrates v1 terminal layouts and clamps git width", () => {
    const s = sanitizeHubState({
      version: 1,
      tabPaths: ["/tmp/proj"],
      names: { "/tmp/proj": "proj" },
      layouts: { "/tmp/proj": { kind: "nope" } },
      active: "/tmp/proj",
      focused: { "/tmp/proj": "pane_1" },
      sessions: { "/tmp/proj": "/home/x/.config/catalyst-code/sessions/abc/chat.jsonl" },
      gitOpen: false,
      gitWidth: 9999,
    });
    expect(s.version).toBe(2);
    expect(s.tabPaths).toEqual(["/tmp/proj"]);
    expect(s.sessions["/tmp/proj"]).toContain("chat.jsonl");
    // Terminal pane fields are dropped.
    expect((s as { layouts?: unknown }).layouts).toBeUndefined();
    expect(s.gitWidth).toBe(560); // clamped
    expect(s.gitOpen).toBe(false);
  });

  test("sanitize drops non-jsonl session paths", () => {
    const s = sanitizeHubState({
      tabPaths: ["/ws"],
      sessions: {
        "/ws": "not-a-session",
        "/other": "/tmp/real.jsonl",
      },
    });
    expect(s.sessions["/ws"]).toBeUndefined();
    expect(s.sessions["/other"]).toBe("/tmp/real.jsonl");
  });

  test("save + load round-trips session paths (cross-device reattach key)", () => {
    expect(loadHubLayout()).toBeNull();
    const state = sanitizeHubState({
      version: 2,
      tabPaths: ["/ws/a"],
      names: { "/ws/a": "a" },
      sessions: {
        "/ws/a": "/home/u/.config/catalyst-code/sessions/deadbeef/2026-01-01.jsonl",
      },
      active: "/ws/a",
      gitOpen: true,
      gitWidth: 300,
    });
    const saved = saveHubLayout("user-1", state);
    expect(saved.userId).toBe("user-1");
    expect(saved.updatedAt).toBeGreaterThan(0);

    const loaded = loadHubLayout();
    expect(loaded).not.toBeNull();
    expect(loaded!.state.tabPaths).toEqual(["/ws/a"]);
    expect(loaded!.state.sessions["/ws/a"]).toBe(
      "/home/u/.config/catalyst-code/sessions/deadbeef/2026-01-01.jsonl",
    );
    expect(loaded!.state.version).toBe(2);

    const raw = JSON.parse(
      readFileSync(join(tmp, ".config", "catalyst-code", "hub-layout.json"), "utf8"),
    );
    expect(raw.userId).toBe("user-1");
    expect(raw.state.sessions["/ws/a"]).toContain("2026-01-01.jsonl");
  });
});
