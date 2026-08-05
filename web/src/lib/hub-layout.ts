// Pure split-tree layout model for the /hub terminal workspace.
//
// A layout is a binary tree: leaves are terminal panes, splits divide space
// "h" (side-by-side, left|right) or "v" (stacked, top/bottom). Every node
// carries a stable id so resize drags and React keys never depend on position.
//
// This module is UI-free and unit-tested (web/src/lib/hub-layout.test.ts).

export type SplitDir = "h" | "v";

export interface LeafNode {
  kind: "leaf";
  /** Pane id — doubles as the persistent terminal session id. */
  id: string;
}

export interface SplitNode {
  kind: "split";
  id: string;
  dir: SplitDir;
  /** Fraction of space given to children[0] (0..1, clamped to sane bounds). */
  ratio: number;
  children: [LayoutNode, LayoutNode];
}

export type LayoutNode = LeafNode | SplitNode;

/** Hard cap so a runaway split/preset cannot spawn unbounded PTYs. */
export const MAX_PANES = 16;

export const MIN_RATIO = 0.1;
export const MAX_RATIO = 0.9;

let splitCounter = 0;

/** A fresh split id (client-local uniqueness is enough). */
export function newSplitId(): string {
  splitCounter += 1;
  return `split_${Date.now().toString(36)}_${splitCounter}`;
}

export function leafNode(id: string): LeafNode {
  return { kind: "leaf", id };
}

function splitNode(dir: SplitDir, ratio: number, a: LayoutNode, b: LayoutNode): SplitNode {
  return { kind: "split", id: newSplitId(), dir, ratio: clampRatio(ratio), children: [a, b] };
}

export function clampRatio(ratio: number): number {
  if (!Number.isFinite(ratio)) return 0.5;
  return Math.min(MAX_RATIO, Math.max(MIN_RATIO, ratio));
}

export function countLeaves(node: LayoutNode): number {
  if (node.kind === "leaf") return 1;
  return countLeaves(node.children[0]) + countLeaves(node.children[1]);
}

/** All leaf ids in visual order (left→right / top→bottom). */
export function leafIds(node: LayoutNode): string[] {
  if (node.kind === "leaf") return [node.id];
  return [...leafIds(node.children[0]), ...leafIds(node.children[1])];
}

export function firstLeafId(node: LayoutNode): string {
  if (node.kind === "leaf") return node.id;
  return firstLeafId(node.children[0]);
}

/** Replace the leaf `leafId` with a split holding the leaf + a new pane.
 *  Returns the new root, or null when the pane cap would be exceeded or the
 *  target leaf does not exist. */
export function splitLeaf(
  root: LayoutNode,
  leafId: string,
  dir: SplitDir,
  newId: string,
): LayoutNode | null {
  if (countLeaves(root) >= MAX_PANES) return null;
  const next = splitLeafInner(root, leafId, dir, newId);
  return next ?? null;
}

function splitLeafInner(
  node: LayoutNode,
  leafId: string,
  dir: SplitDir,
  newId: string,
): LayoutNode | null {
  if (node.kind === "leaf") {
    if (node.id !== leafId) return null;
    return splitNode(dir, 0.5, node, leafNode(newId));
  }
  const left = splitLeafInner(node.children[0], leafId, dir, newId);
  if (left) return { ...node, children: [left, node.children[1]] };
  const right = splitLeafInner(node.children[1], leafId, dir, newId);
  if (right) return { ...node, children: [node.children[0], right] };
  return null;
}

/** Remove the leaf `leafId`, collapsing its parent split onto the sibling.
 *  Returns the new root, or null when the last pane was closed. */
export function closeLeaf(root: LayoutNode, leafId: string): LayoutNode | null {
  if (root.kind === "leaf") return root.id === leafId ? null : root;
  const [a, b] = root.children;
  if (a.kind === "leaf" && a.id === leafId) return b;
  if (b.kind === "leaf" && b.id === leafId) return a;
  const nextA = closeLeaf(a, leafId);
  if (nextA !== a) return nextA === null ? b : { ...root, children: [nextA, b] };
  const nextB = closeLeaf(b, leafId);
  if (nextB !== b) return nextB === null ? a : { ...root, children: [a, nextB] };
  return root;
}

/** Update a split's ratio (divider drag). Returns the new root. */
export function setRatio(root: LayoutNode, splitId: string, ratio: number): LayoutNode {
  if (root.kind === "leaf") return root;
  if (root.id === splitId) return { ...root, ratio: clampRatio(ratio) };
  const a = setRatio(root.children[0], splitId, ratio);
  const b = setRatio(root.children[1], splitId, ratio);
  if (a === root.children[0] && b === root.children[1]) return root;
  return { ...root, children: [a, b] };
}

/** Swap a leaf's id in place (pane restart: new terminal session, same slot).
 *  Returns the new root, or null when the leaf does not exist. */
export function replaceLeafId(root: LayoutNode, oldId: string, newId: string): LayoutNode | null {
  if (root.kind === "leaf") return root.id === oldId ? leafNode(newId) : null;
  const a = replaceLeafId(root.children[0], oldId, newId);
  if (a) return { ...root, children: [a, root.children[1]] };
  const b = replaceLeafId(root.children[1], oldId, newId);
  if (b) return { ...root, children: [root.children[0], b] };
  return null;
}

/** Even n-way composition: the first child gets 1/n of the space and the
 *  remainder is nested recursively, so all n children end up equal-sized. */
function evenSplit(dir: SplitDir, nodes: LayoutNode[]): LayoutNode {
  if (nodes.length === 1) return nodes[0];
  const [head, ...rest] = nodes;
  return splitNode(dir, 1 / nodes.length, head, evenSplit(dir, rest));
}

/** Build a rows×cols grid of fresh leaves. `makeId` mints pane ids in visual
 *  order (row-major). 1×1 returns a single leaf. */
export function gridLayout(rows: number, cols: number, makeId: () => string): LayoutNode {
  const r = Math.max(1, Math.min(4, Math.floor(rows)));
  const c = Math.max(1, Math.min(4, Math.floor(cols)));
  if (r * c > MAX_PANES) throw new Error(`grid ${r}x${c} exceeds the ${MAX_PANES}-pane cap`);
  const rowNodes = Array.from({ length: r }, () =>
    evenSplit("h", Array.from({ length: c }, () => leafNode(makeId()))),
  );
  return evenSplit("v", rowNodes);
}

/** Like gridLayout, but with caller-supplied leaf ids (row-major). Lets the
 *  hub apply a preset WITHOUT killing running terminals: existing panes keep
 *  their ids (and PTYs), and the caller terminates only the surplus. */
export function gridLayoutWithIds(rows: number, cols: number, ids: string[]): LayoutNode {
  const r = Math.max(1, Math.min(4, Math.floor(rows)));
  const c = Math.max(1, Math.min(4, Math.floor(cols)));
  if (ids.length !== r * c) throw new Error(`need ${r * c} ids, got ${ids.length}`);
  let cursor = 0;
  const rowNodes = Array.from({ length: r }, () =>
    evenSplit("h", Array.from({ length: c }, () => leafNode(ids[cursor++]))),
  );
  return evenSplit("v", rowNodes);
}

export type HubPreset =
  | "single"
  | "columns2"
  | "rows2"
  | "grid2x2"
  | "grid3x3"
  | "grid4x4";

export const HUB_PRESETS: Array<{ id: HubPreset; label: string; rows: number; cols: number }> = [
  { id: "single", label: "1", rows: 1, cols: 1 },
  { id: "columns2", label: "1×2", rows: 1, cols: 2 },
  { id: "rows2", label: "2×1", rows: 2, cols: 1 },
  { id: "grid2x2", label: "2×2", rows: 2, cols: 2 },
  { id: "grid3x3", label: "3×3", rows: 3, cols: 3 },
  { id: "grid4x4", label: "4×4", rows: 4, cols: 4 },
];

export function presetLayout(preset: HubPreset, makeId: () => string): LayoutNode {
  const def = HUB_PRESETS.find((p) => p.id === preset) ?? HUB_PRESETS[0];
  return gridLayout(def.rows, def.cols, makeId);
}

/** Preset shape as rows/cols (falls back to single). */
export function presetShape(preset: HubPreset): { rows: number; cols: number } {
  const def = HUB_PRESETS.find((p) => p.id === preset) ?? HUB_PRESETS[0];
  return { rows: def.rows, cols: def.cols };
}

/** Structural validation for layouts restored from localStorage. Returns the
 *  sanitized tree (ratios clamped, ids coerced) or null when unusable. */
export function validateLayout(value: unknown): LayoutNode | null {
  if (!value || typeof value !== "object") return null;
  const node = value as Record<string, unknown>;
  if (node.kind === "leaf") {
    return typeof node.id === "string" && node.id.length > 0 && node.id.length <= 128
      ? leafNode(node.id)
      : null;
  }
  if (node.kind === "split") {
    const dir = node.dir === "v" ? "v" : node.dir === "h" ? "h" : null;
    if (!dir) return null;
    if (!Array.isArray(node.children) || node.children.length !== 2) return null;
    const a = validateLayout(node.children[0]);
    const b = validateLayout(node.children[1]);
    if (!a || !b) return null;
    const split = splitNode(dir, typeof node.ratio === "number" ? node.ratio : 0.5, a, b);
    return typeof node.id === "string" && node.id.length > 0 ? { ...split, id: node.id } : split;
  }
  return null;
}
