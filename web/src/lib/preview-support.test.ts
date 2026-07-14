import { describe, expect, test } from "bun:test";
import { canPreviewFile, previewExtension } from "./preview-support";

describe("preview file support", () => {
  test("accepts every file type rendered by the Preview panel", () => {
    for (const path of [
      "README.md",
      "guide.markdown",
      "public/index.HTML",
      "legacy.htm",
      "manual.pdf",
      "logo.svg",
      "photo.png",
      "photo.jpg",
      "photo.jpeg",
      "animation.gif",
      "screenshot.webp",
    ]) {
      expect(canPreviewFile(path)).toBe(true);
    }
  });

  test("rejects files the in-app renderer cannot display", () => {
    for (const path of ["main.ts", "data.json", "notes.txt", "image.bmp", "Makefile"]) {
      expect(canPreviewFile(path)).toBe(false);
    }
  });

  test("handles Windows paths and URL suffixes", () => {
    expect(previewExtension("docs\\guide.MD?raw=1#intro")).toBe("md");
  });
});
