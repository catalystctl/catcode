import { describe, expect, test } from "bun:test";
import {
  MOUSE_BTN,
  clientToCell,
  domButtonToXterm,
  encodeSgrMouse,
  encodeSgrMouseMotion,
  encodeWheelReports,
  modsFromEvent,
} from "./terminal-mouse";

describe("encodeSgrMouse", () => {
  test("left press at 1,1", () => {
    expect(encodeSgrMouse(MOUSE_BTN.LEFT, 1, 1, false)).toBe("\x1b[<0;1;1M");
  });

  test("left release", () => {
    expect(encodeSgrMouse(MOUSE_BTN.LEFT, 10, 5, true)).toBe("\x1b[<0;10;5m");
  });

  test("right press with ctrl+shift", () => {
    expect(
      encodeSgrMouse(MOUSE_BTN.RIGHT, 3, 4, false, { ctrl: true, shift: true }),
    ).toBe("\x1b[<22;3;4M"); // 2 + 4 + 16
  });

  test("clamps fractional / zero coords to ≥1", () => {
    expect(encodeSgrMouse(0, 0, -2, false)).toBe("\x1b[<0;1;1M");
  });

  test("motion sets +32", () => {
    expect(encodeSgrMouseMotion(MOUSE_BTN.LEFT, 8, 2)).toBe("\x1b[<32;8;2M");
  });

  test("wheel up/down", () => {
    expect(encodeSgrMouse(MOUSE_BTN.WHEEL_UP, 1, 1, false)).toBe("\x1b[<64;1;1M");
    expect(encodeSgrMouse(MOUSE_BTN.WHEEL_DOWN, 1, 1, false)).toBe("\x1b[<65;1;1M");
  });
});

describe("clientToCell", () => {
  const metrics = {
    cols: 80,
    rows: 24,
    rect: { left: 100, top: 50, width: 800, height: 480 },
  };

  test("top-left cell", () => {
    expect(clientToCell(100, 50, metrics)).toEqual({ col: 1, row: 1 });
  });

  test("inside grid", () => {
    // cellW=10, cellH=20 → col 5 is x in [140,150), row 3 is y in [90,110)
    expect(clientToCell(145, 95, metrics)).toEqual({ col: 5, row: 3 });
  });

  test("clamps past bottom-right", () => {
    expect(clientToCell(9999, 9999, metrics)).toEqual({ col: 80, row: 24 });
  });

  test("degenerate rect → 1,1", () => {
    expect(
      clientToCell(0, 0, { cols: 80, rows: 24, rect: { left: 0, top: 0, width: 0, height: 0 } }),
    ).toEqual({ col: 1, row: 1 });
  });
});

describe("dom helpers", () => {
  test("domButtonToXterm", () => {
    expect(domButtonToXterm(0)).toBe(0);
    expect(domButtonToXterm(1)).toBe(1);
    expect(domButtonToXterm(2)).toBe(2);
    expect(domButtonToXterm(3)).toBe(0);
  });

  test("modsFromEvent", () => {
    expect(
      modsFromEvent({ shiftKey: true, altKey: false, metaKey: true, ctrlKey: false }),
    ).toEqual({ shift: true, alt: false, meta: true, ctrl: false });
  });
});

describe("encodeWheelReports", () => {
  test("emits wheel-down for positive deltaY", () => {
    const r = encodeWheelReports(40, 0, 2, 3);
    expect(r.length).toBeGreaterThanOrEqual(1);
    expect(r[0]).toBe("\x1b[<65;2;3M");
  });

  test("emits wheel-up for negative deltaY", () => {
    const r = encodeWheelReports(-10, 0, 1, 1);
    expect(r[0]).toBe("\x1b[<64;1;1M");
  });

  test("caps notch count at 5", () => {
    const r = encodeWheelReports(10_000, 0, 1, 1);
    expect(r.length).toBe(5);
  });
});
