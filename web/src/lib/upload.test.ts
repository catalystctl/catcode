import { describe, expect, test } from "bun:test";
import {
  filesToUploadFiles,
  findConflicts,
  keepBothName,
  isSafeUploadName,
} from "./upload";

/** Build a minimal File-like object for testing path-flattening logic. */
function fakeFile(name: string, webkitRelativePath?: string): File {
  const file = {
    name,
    size: 0,
    type: "",
    lastModified: 0,
    arrayBuffer: () => Promise.resolve(new ArrayBuffer(0)),
  } as unknown as File;
  if (webkitRelativePath) {
    (file as File & { webkitRelativePath?: string }).webkitRelativePath = webkitRelativePath;
  }
  return file;
}

describe("upload helper", () => {
  test("filesToUploadFiles keeps simple file names", () => {
    const out = filesToUploadFiles([fakeFile("a.txt"), fakeFile("b.png")]);
    expect(out.map((f) => f.relPath)).toEqual(["a.txt", "b.png"]);
    expect(out.map((f) => f.name)).toEqual(["a.txt", "b.png"]);
  });

  test("filesToUploadFiles flattens folder uploads, dropping the root folder", () => {
    const out = filesToUploadFiles([
      fakeFile("site", "site/index.html"),
      fakeFile("logo.png", "site/assets/logo.png"),
      fakeFile("readme.md", "site/readme.md"),
    ]);
    expect(out.map((f) => f.relPath).sort()).toEqual([
      "assets/logo.png",
      "index.html",
      "readme.md",
    ]);
  });

  test("findConflicts returns only files whose relPath is in the existing set", () => {
    const files = [
      { file: fakeFile("a.txt"), relPath: "a.txt", name: "a.txt" },
      { file: fakeFile("b.txt"), relPath: "b.txt", name: "b.txt" },
      { file: fakeFile("c.txt"), relPath: "sub/c.txt", name: "c.txt" },
    ];
    const existing = new Set(["a.txt", "sub/c.txt"]);
    const conflicts = findConflicts(files, existing);
    expect(conflicts.map((c) => c.relPath).sort()).toEqual(["a.txt", "sub/c.txt"]);
  });

  test("findConflicts returns empty when nothing collides", () => {
    const files = [{ file: fakeFile("a.txt"), relPath: "a.txt", name: "a.txt" }];
    expect(findConflicts(files, new Set(["b.txt"]))).toEqual([]);
  });

  test("keepBothName appends (n) before the extension", () => {
    const existing = new Set(["report.pdf"]);
    expect(keepBothName("report.pdf", existing)).toBe("report (1).pdf");
  });

  test("keepBothName increments to avoid repeated collisions", () => {
    const existing = new Set(["report.pdf", "report (1).pdf"]);
    expect(keepBothName("report.pdf", existing)).toBe("report (2).pdf");
  });

  test("keepBothName works for extensionless names", () => {
    const existing = new Set(["README"]);
    expect(keepBothName("README", existing)).toBe("README (1)");
  });

  test("keepBothName preserves the subdirectory prefix", () => {
    const existing = new Set(["docs/index.html"]);
    expect(keepBothName("docs/index.html", existing)).toBe("docs/index (1).html");
  });

  test("keepBothName registers the chosen name so successive keeps don't re-collide", () => {
    const existing = new Set(["a.txt"]);
    const first = keepBothName("a.txt", existing);
    const second = keepBothName("a.txt", existing);
    expect(first).toBe("a (1).txt");
    expect(second).toBe("a (2).txt");
  });

  test("isSafeUploadName rejects traversal and absolute paths", () => {
    expect(isSafeUploadName("../escape.txt")).toBe(false);
    expect(isSafeUploadName("sub/../../escape.txt")).toBe(false);
    expect(isSafeUploadName("/etc/passwd")).toBe(false);
    expect(isSafeUploadName("ok/file.txt")).toBe(true);
    expect(isSafeUploadName("file.txt")).toBe(true);
    expect(isSafeUploadName("")).toBe(false);
    expect(isSafeUploadName("a\0b")).toBe(false);
  });
});
