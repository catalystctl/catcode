import { describe, expect, test } from "bun:test";
import { allowWorkspaceSwitch, dirtyWorkspaceSwitchMessage } from "./workspace-switch";

describe("workspace switch guard", () => {
  test("switches immediately when no editor is dirty", () => {
    let prompted = false;
    expect(
      allowWorkspaceSwitch("/a", "/b", [{ label: "clean.ts", dirty: false }], () => {
        prompted = true;
        return false;
      }),
    ).toBe(true);
    expect(prompted).toBe(false);
  });

  test("cancellation blocks a dirty project switch", () => {
    let prompt = "";
    const allowed = allowWorkspaceSwitch(
      "/a",
      "/b",
      [{ label: "unsaved.ts", dirty: true }],
      (message) => {
        prompt = message;
        return false;
      },
    );
    expect(allowed).toBe(false);
    expect(prompt).toContain("unsaved.ts");
  });

  test("explicit discard permits the switch and summarizes long dirty lists", () => {
    const tabs = ["a", "b", "c", "d", "clean"].map((label, index) => ({
      label,
      dirty: index < 4,
    }));
    expect(dirtyWorkspaceSwitchMessage(tabs)).toBe(
      "Discard unsaved changes in a, b, c and 1 more and switch projects?",
    );
    expect(allowWorkspaceSwitch("/a", "/b", tabs, () => true)).toBe(true);
  });

  test("empty and current-workspace targets are no-ops", () => {
    expect(allowWorkspaceSwitch("/a", "", [], () => true)).toBe(false);
    expect(allowWorkspaceSwitch("/a", "/a", [], () => true)).toBe(false);
  });
});
