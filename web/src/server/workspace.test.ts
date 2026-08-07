import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { confinePath, confinePathReal } from "./workspace";

describe("confinePath", () => {
  test("allows workspace-relative paths", () => {
    expect(confinePath("/proj", "src/main.ts").replace(/\\/g, "/")).toBe("/proj/src/main.ts");
    expect(confinePath("/proj", ".").replace(/\\/g, "/")).toBe("/proj");
    expect(confinePath("/proj", "a/../b").replace(/\\/g, "/")).toBe("/proj/b");
  });

  test("rejects parent traversal", () => {
    expect(() => confinePath("/proj", "../etc")).toThrow("path outside workspace");
    expect(() => confinePath("/proj", "foo/../../etc")).toThrow("path outside workspace");
  });

  test("rejects absolute client paths (do not reinterpret as relative)", () => {
    expect(() => confinePath("/proj", "/etc")).toThrow("path outside workspace");
    expect(() => confinePath("/proj", "/etc/passwd")).toThrow("path outside workspace");
  });

  test("rejects empty path", () => {
    expect(() => confinePath("/proj", "")).toThrow("path outside workspace");
  });
});

describe("confinePathReal", () => {
  test("rejects symlink that escapes the workspace", () => {
    const root = mkdtempSync(join(tmpdir(), "confine-"));
    try {
      const ws = join(root, "ws");
      mkdirSync(ws);
      // link → parent of workspace (escape)
      symlinkSync(root, join(ws, "out"));
      expect(() => confinePathReal(ws, "out")).toThrow("path outside workspace");
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test("allows real paths that stay inside the workspace", () => {
    const root = mkdtempSync(join(tmpdir(), "confine-in-"));
    try {
      const ws = join(root, "ws");
      mkdirSync(ws);
      writeFileSync(join(ws, "file.txt"), "ok");
      const real = confinePathReal(ws, "file.txt");
      expect(real.endsWith("file.txt")).toBe(true);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
