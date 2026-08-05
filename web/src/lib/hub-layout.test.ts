import { describe, expect, test } from "bun:test";
import {
  MAX_PANES,
  clampRatio,
  closeLeaf,
  countLeaves,
  firstLeafId,
  gridLayout,
  gridLayoutWithIds,
  leafIds,
  leafNode,
  presetLayout,
  presetShape,
  replaceLeafId,
  setRatio,
  splitLeaf,
  validateLayout,
} from "./hub-layout";

let n = 0;
const makeId = () => `pane_${++n}`;

describe("hub layout presets", () => {
  test("single preset is one leaf", () => {
    const root = presetLayout("single", makeId);
    expect(root.kind).toBe("leaf");
    expect(countLeaves(root)).toBe(1);
  });

  test.each([
    ["columns2", 2],
    ["rows2", 2],
    ["grid2x2", 4],
    ["grid3x3", 9],
    ["grid4x4", 16],
  ] as const)("%s creates %i panes", (preset, want) => {
    const root = presetLayout(preset, makeId);
    expect(countLeaves(root)).toBe(want);
  });

  test("grid ratios give every pane an equal share", () => {
    // 4 columns: ratios 1/4, 1/3, 1/2 nested → each pane ends up with 1/4.
    const root = gridLayout(1, 4, makeId);
    expect(root.kind).toBe("split");
    if (root.kind !== "split") return;
    expect(root.ratio).toBeCloseTo(0.25);
    const rest = root.children[1];
    expect(rest.kind).toBe("split");
    if (rest.kind !== "split") return;
    expect(rest.ratio).toBeCloseTo(1 / 3);
  });

  test("grid dimensions are clamped to 4x4", () => {
    const root = gridLayout(9, 9, makeId);
    expect(countLeaves(root)).toBe(16);
  });

  test("leaf ids come out in visual (row-major) order", () => {
    const ids: string[] = [];
    const root = gridLayout(2, 2, () => {
      const id = `p${ids.length}`;
      ids.push(id);
      return id;
    });
    expect(leafIds(root)).toEqual(ids);
  });
});

describe("split + close operations", () => {
  test("splitLeaf replaces the target leaf with a split", () => {
    const root = leafNode("a");
    const next = splitLeaf(root, "a", "h", "b");
    expect(next).not.toBeNull();
    expect(countLeaves(next!)).toBe(2);
    expect(leafIds(next!)).toEqual(["a", "b"]);
  });

  test("splitLeaf into a nested tree finds the right leaf", () => {
    let root = splitLeaf(leafNode("a"), "a", "h", "b")!;
    root = splitLeaf(root, "b", "v", "c")!;
    expect(leafIds(root)).toEqual(["a", "b", "c"]);
    expect(countLeaves(root)).toBe(3);
  });

  test("splitLeaf returns null for unknown leaves", () => {
    expect(splitLeaf(leafNode("a"), "zzz", "h", "b")).toBeNull();
  });

  test("splitLeaf enforces the pane cap", () => {
    let root = presetLayout("grid4x4", makeId);
    const anyLeaf = firstLeafId(root);
    expect(splitLeaf(root, anyLeaf, "h", "overflow")).toBeNull();
    expect(countLeaves(root)).toBe(MAX_PANES);
  });

  test("closeLeaf collapses onto the sibling", () => {
    const root = splitLeaf(leafNode("a"), "a", "h", "b")!;
    const afterClose = closeLeaf(root, "b");
    expect(afterClose).not.toBeNull();
    expect(afterClose!.kind).toBe("leaf");
    expect(afterClose!.kind === "leaf" && afterClose!.id).toBe("a");
  });

  test("closeLeaf keeps untouched subtrees intact", () => {
    let root = splitLeaf(leafNode("a"), "a", "h", "b")!;
    root = splitLeaf(root, "b", "v", "c")!;
    const next = closeLeaf(root, "a")!;
    expect(leafIds(next)).toEqual(["b", "c"]);
  });

  test("closing the last pane returns null", () => {
    expect(closeLeaf(leafNode("a"), "a")).toBeNull();
    const root = splitLeaf(leafNode("a"), "a", "h", "b")!;
    const one = closeLeaf(root, "b")!;
    expect(closeLeaf(one, "a")).toBeNull();
  });

  test("closeLeaf on an unknown leaf is a no-op", () => {
    const root = leafNode("a");
    expect(closeLeaf(root, "zzz")).toBe(root);
  });
});

describe("setRatio", () => {
  test("updates the targeted split and clamps", () => {
    const root = splitLeaf(leafNode("a"), "a", "h", "b")!;
    if (root.kind !== "split") throw new Error("expected split");
    const next = setRatio(root, root.id, 0.99);
    expect(next.kind).toBe("split");
    expect(next.kind === "split" && next.ratio).toBe(clampRatio(0.99));
  });

  test("ignores unknown split ids", () => {
    const root = leafNode("a");
    expect(setRatio(root, "nope", 0.7)).toBe(root);
  });
});

describe("gridLayoutWithIds + presetShape", () => {
  test("builds a grid from caller ids in row-major order", () => {
    const root = gridLayoutWithIds(2, 2, ["a", "b", "c", "d"]);
    expect(leafIds(root)).toEqual(["a", "b", "c", "d"]);
  });

  test("rejects mismatched id counts", () => {
    expect(() => gridLayoutWithIds(2, 2, ["a", "b"])).toThrow();
  });

  test("presetShape matches the preset table", () => {
    expect(presetShape("grid4x4")).toEqual({ rows: 4, cols: 4 });
    expect(presetShape("columns2")).toEqual({ rows: 1, cols: 2 });
  });
});

describe("replaceLeafId", () => {
  test("swaps the id in place, preserving structure", () => {
    let root = splitLeaf(leafNode("a"), "a", "h", "b")!;
    root = splitLeaf(root, "b", "v", "c")!;
    const next = replaceLeafId(root, "b", "b2");
    expect(next).not.toBeNull();
    expect(leafIds(next!)).toEqual(["a", "b2", "c"]);
  });

  test("returns null for an unknown leaf", () => {
    expect(replaceLeafId(leafNode("a"), "zzz", "n")).toBeNull();
  });
});

describe("validateLayout", () => {
  test("round-trips a serialized layout", () => {
    const root = splitLeaf(leafNode("a"), "a", "v", "b")!;
    const restored = validateLayout(JSON.parse(JSON.stringify(root)));
    expect(restored).not.toBeNull();
    expect(leafIds(restored!)).toEqual(["a", "b"]);
  });

  test("rejects garbage", () => {
    expect(validateLayout(null)).toBeNull();
    expect(validateLayout({})).toBeNull();
    expect(validateLayout({ kind: "leaf" })).toBeNull();
    expect(validateLayout({ kind: "split", dir: "x", children: [] })).toBeNull();
    expect(validateLayout({ kind: "leaf", id: "" })).toBeNull();
  });

  test("clamps persisted ratios back into bounds", () => {
    const raw = {
      kind: "split",
      id: "s1",
      dir: "h",
      ratio: 5,
      children: [
        { kind: "leaf", id: "a" },
        { kind: "leaf", id: "b" },
      ],
    };
    const restored = validateLayout(raw);
    expect(restored).not.toBeNull();
    expect(restored!.kind === "split" && restored!.ratio).toBeLessThanOrEqual(0.9);
  });
});
