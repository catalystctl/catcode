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

describe("hub layout store", () => {
  test("default state is empty", () => {
    const s = defaultHubState();
    expect(s.tabPaths).toEqual([]);
    expect(s.active).toBeNull();
    expect(s.gitOpen).toBe(true);
  });

  test("sanitize recovers corrupt layouts with a fresh leaf", () => {
    const s = sanitizeHubState(
      {
        version: 1,
        tabPaths: ["/tmp/proj"],
        names: { "/tmp/proj": "proj" },
        layouts: { "/tmp/proj": { kind: "nope" } },
        active: "/tmp/proj",
        focused: {},
        gitOpen: false,
        gitWidth: 9999,
      },
      () => "pane_fixed",
    );
    expect(s.tabPaths).toEqual(["/tmp/proj"]);
    expect(s.layouts["/tmp/proj"]).toEqual({ kind: "leaf", id: "pane_fixed" });
    expect(s.gitWidth).toBe(560); // clamped
    expect(s.gitOpen).toBe(false);
  });

  test("save + load round-trips pane ids (cross-device reattach key)", () => {
    expect(loadHubLayout()).toBeNull();
    const state = sanitizeHubState({
      version: 1,
      tabPaths: ["/ws/a"],
      names: { "/ws/a": "a" },
      layouts: {
        "/ws/a": {
          kind: "split",
          id: "split_1",
          dir: "h",
          ratio: 0.4,
          children: [
            { kind: "leaf", id: "hub_abc_1" },
            { kind: "leaf", id: "hub_abc_2" },
          ],
        },
      },
      active: "/ws/a",
      focused: { "/ws/a": "hub_abc_2" },
      gitOpen: true,
      gitWidth: 300,
    });
    const saved = saveHubLayout("user-1", state);
    expect(saved.userId).toBe("user-1");
    expect(saved.updatedAt).toBeGreaterThan(0);

    const loaded = loadHubLayout();
    expect(loaded).not.toBeNull();
    expect(loaded!.state.tabPaths).toEqual(["/ws/a"]);
    expect(loaded!.state.layouts["/ws/a"]).toEqual(state.layouts["/ws/a"]);
    expect(loaded!.state.focused["/ws/a"]).toBe("hub_abc_2");

    // File lands under the temp HOME config dir.
    const raw = JSON.parse(
      readFileSync(join(tmp, ".config", "catalyst-code", "hub-layout.json"), "utf8"),
    );
    expect(raw.userId).toBe("user-1");
    expect(raw.state.layouts["/ws/a"].children[0].id).toBe("hub_abc_1");
  });
});
