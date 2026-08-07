// web/src/lib/terminal-mouse.ts
//
// Ghostty-web 0.4 never encodes application mouse tracking (DECSET 1000/1002/
// 1003 + SGR 1006) into onData — hasMouseTracking() is true after the TUI
// enables it, but clicks only drive local SelectionManager / scrollbar /
// alt-screen cursor-key wheel. Catcode's clickable chrome therefore never
// receives CSI mouse reports over the hub PTY.
//
// This module is the pure encode + cell-math half; terminal.tsx installs the
// DOM listeners and gates on term.hasMouseTracking().

/** X10/SGR button codes (before motion/mod bit flags). */
export const MOUSE_BTN = {
  LEFT: 0,
  MIDDLE: 1,
  RIGHT: 2,
  /** Released / no-button motion base (OR'd with 32 for motion). */
  RELEASE: 3,
  WHEEL_UP: 64,
  WHEEL_DOWN: 65,
  WHEEL_LEFT: 66,
  WHEEL_RIGHT: 67,
} as const;

export interface MouseMods {
  shift?: boolean;
  /** Alt / Option / Meta (mod bit 8). */
  meta?: boolean;
  /** Alt is also reported as meta in xterm SGR for many terminals. */
  alt?: boolean;
  ctrl?: boolean;
}

/**
 * Encode one SGR (1006) mouse report.
 * Coordinates are 1-based cell positions (xterm convention).
 * `release` selects the final byte: `M` press/motion, `m` release.
 */
export function encodeSgrMouse(
  button: number,
  col: number,
  row: number,
  release: boolean,
  mods: MouseMods = {},
): string {
  const cx = Math.max(1, Math.floor(col));
  const cy = Math.max(1, Math.floor(row));
  let cb = button;
  if (mods.shift) cb += 4;
  if (mods.meta || mods.alt) cb += 8;
  if (mods.ctrl) cb += 16;
  return `\x1b[<${cb};${cx};${cy}${release ? "m" : "M"}`;
}

/** Motion reports set bit 5 (+32) on the button code. */
export function encodeSgrMouseMotion(
  button: number,
  col: number,
  row: number,
  mods: MouseMods = {},
): string {
  return encodeSgrMouse(button + 32, col, row, false, mods);
}

export interface CellMetrics {
  cols: number;
  rows: number;
  /** Canvas (or terminal surface) bounding rect in CSS pixels. */
  rect: { left: number; top: number; width: number; height: number };
}

/**
 * Map a client-pixel point to 1-based cell (col, row).
 * Clamps into the visible grid so edge drags still hit the border cell.
 */
export function clientToCell(
  clientX: number,
  clientY: number,
  metrics: CellMetrics,
): { col: number; row: number } {
  const { cols, rows, rect } = metrics;
  if (cols < 1 || rows < 1 || rect.width <= 0 || rect.height <= 0) {
    return { col: 1, row: 1 };
  }
  const cellW = rect.width / cols;
  const cellH = rect.height / rows;
  const col = Math.min(cols, Math.max(1, Math.floor((clientX - rect.left) / cellW) + 1));
  const row = Math.min(rows, Math.max(1, Math.floor((clientY - rect.top) / cellH) + 1));
  return { col, row };
}

/** DOM button → xterm button (0/1/2). Auxiliary buttons map to left. */
export function domButtonToXterm(button: number): number {
  if (button === 1) return MOUSE_BTN.MIDDLE;
  if (button === 2) return MOUSE_BTN.RIGHT;
  return MOUSE_BTN.LEFT;
}

export function modsFromEvent(e: {
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
}): MouseMods {
  return {
    shift: e.shiftKey,
    alt: e.altKey,
    meta: e.metaKey,
    ctrl: e.ctrlKey,
  };
}

/**
 * Wheel → one or more SGR wheel button reports.
 * Browsers report large pixel deltas; we emit a bounded number of notches
 * (same spirit as ghostty's alt-screen wheel path, max 5).
 */
export function encodeWheelReports(
  deltaY: number,
  deltaX: number,
  col: number,
  row: number,
  mods: MouseMods = {},
): string[] {
  const out: string[] = [];
  const yNotches = Math.min(5, Math.max(1, Math.round(Math.abs(deltaY) / 33) || (deltaY === 0 ? 0 : 1)));
  const xNotches = Math.min(5, Math.max(1, Math.round(Math.abs(deltaX) / 33) || (deltaX === 0 ? 0 : 1)));
  if (deltaY !== 0) {
    const btn = deltaY < 0 ? MOUSE_BTN.WHEEL_UP : MOUSE_BTN.WHEEL_DOWN;
    for (let i = 0; i < yNotches; i++) out.push(encodeSgrMouse(btn, col, row, false, mods));
  }
  if (deltaX !== 0 && Math.abs(deltaX) > Math.abs(deltaY)) {
    const btn = deltaX < 0 ? MOUSE_BTN.WHEEL_LEFT : MOUSE_BTN.WHEEL_RIGHT;
    for (let i = 0; i < xNotches; i++) out.push(encodeSgrMouse(btn, col, row, false, mods));
  }
  return out;
}
