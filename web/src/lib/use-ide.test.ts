import { describe, expect, test } from "bun:test";
import { sanitizePersistedLayout } from "./use-ide";

describe("persisted IDE layout recovery", () => {
  test("repairs oversized dimensions so the main editor remains reachable", () => {
    const restored = sanitizePersistedLayout(
      {
        sidebarCollapsed: false,
        sidebarWidth: 99_999,
        copilotWidth: 99_999,
        bottomPanelHeight: 99_999,
        panelLocations: { chat: "right" },
        panelVisibility: { chat: true },
      },
      { width: 1366, height: 768 },
    );

    expect(restored.sidebarWidth).toBe(640);
    expect(restored.copilotWidth).toBe(434);
    expect(restored.bottomPanelHeight).toBe(508);
    expect(
      48 + (restored.sidebarWidth ?? 0) + (restored.copilotWidth ?? 0),
    ).toBeLessThanOrEqual(1366 - 244);
  });

  test("rejects unknown panels, docks, non-finite sizes, and stale terminals", () => {
    const restored = sanitizePersistedLayout(
      {
        activePanel: "unknown",
        sidebarWidth: Number.NaN,
        panelLocations: { chat: "offscreen", terminal: "bottom", unknown: "main" },
        panelVisibility: { chat: "yes", terminal: true },
        activeDockPanels: { right: "unknown", bottom: "chat" },
        terminals: [
          { id: "../bad", title: "bad", cwd: "", alive: true, exitCode: null },
          { id: "dead", title: "dead", cwd: "", alive: false, exitCode: 0 },
          { id: "live_1", title: "live", cwd: "", alive: true, exitCode: null },
          { id: "live_1", title: "duplicate", cwd: "/other", alive: true, exitCode: null },
        ],
        activeTerminalId: "dead",
        expandedDirs: ["src", "src", 42],
      },
      { width: 1024, height: 700 },
    );

    expect(restored.activePanel).toBe("explorer");
    expect(restored.sidebarWidth).toBe(256);
    expect(restored.panelLocations?.chat).toBe("right");
    expect(restored.activeDockPanels?.bottom).toBe("terminal");
    expect(restored.terminals?.map((terminal) => terminal.id)).toEqual(["live_1"]);
    expect(restored.activeTerminalId).toBe("live_1");
    expect(restored.expandedDirs).toEqual(["src"]);
  });

  test("repairs contradictory dock selection and partial older schemas", () => {
    const restored = sanitizePersistedLayout(
      {
        panelLocations: { terminal: "bottom", chat: "right" },
        panelVisibility: { terminal: true, chat: false },
        activeDockPanels: { bottom: "chat", right: "terminal" },
      },
      { width: 1280, height: 720 },
    );

    expect(restored.activeDockPanels).toEqual({
      left: null,
      right: null,
      bottom: "terminal",
      main: null,
    });
    expect(restored.panelLocations?.screen).toBe("main");
    expect(restored.panelVisibility?.screen).toBe(false);
  });

  test("clamps a desktop layout for a smaller future viewport", () => {
    const restored = sanitizePersistedLayout(
      {
        sidebarCollapsed: false,
        sidebarWidth: 640,
        copilotWidth: 900,
        panelLocations: { chat: "right" },
        panelVisibility: { chat: true },
      },
      { width: 1024, height: 600 },
    );

    expect(restored.sidebarWidth).toBe(412);
    expect(restored.copilotWidth).toBe(320);
    expect(restored.bottomPanelHeight).toBeLessThanOrEqual(340);
  });
});
