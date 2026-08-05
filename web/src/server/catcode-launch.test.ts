import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { CATCODE_BIN_ENV, CATCODE_NOT_FOUND_ERROR, resolveCatcodeBinary } from "./catcode-launch";

let tmp: string;

beforeEach(() => {
  tmp = mkdtempSync(join(tmpdir(), "catcode-launch-"));
});

afterEach(() => {
  rmSync(tmp, { recursive: true, force: true });
});

function fakeBin(name: string, mode = 0o755): string {
  const abs = join(tmp, name);
  writeFileSync(abs, "#!/bin/sh\nexit 0\n", { mode });
  chmodSync(abs, mode); // explicit — umask can strip bits from writeFileSync's mode
  return abs;
}

describe("resolveCatcodeBinary", () => {
  test("finds catcode via a PATH walk (posix)", () => {
    const bin = fakeBin("catcode");
    const env = { PATH: `${join(tmp, "empty")}${delimiter}${tmp}` } as Record<string, string | undefined>;
    expect(resolveCatcodeBinary(env, "linux", tmp)).toBe(bin);
  });

  test("skips non-executable files on posix", () => {
    fakeBin("catcode", 0o644);
    const env = { PATH: tmp } as Record<string, string | undefined>;
    expect(resolveCatcodeBinary(env, "linux", tmp)).toBeNull();
  });

  test("finds catcode.exe on win32 without an exec bit", () => {
    const bin = fakeBin("catcode.exe", 0o644);
    const env = { Path: tmp } as Record<string, string | undefined>;
    expect(resolveCatcodeBinary(env, "win32", tmp)).toBe(bin);
  });

  test("env override wins over PATH", () => {
    const override = fakeBin("catcode-custom");
    fakeBin("catcode");
    const env = { PATH: tmp, [CATCODE_BIN_ENV]: override } as Record<string, string | undefined>;
    expect(resolveCatcodeBinary(env, "linux", tmp)).toBe(override);
  });

  test("a bad env override fails fast instead of falling back", () => {
    fakeBin("catcode");
    const env = { PATH: tmp, [CATCODE_BIN_ENV]: join(tmp, "missing") } as Record<string, string | undefined>;
    expect(resolveCatcodeBinary(env, "linux", tmp)).toBeNull();
  });

  test("relative env override resolves against the server cwd", () => {
    const abs = fakeBin("catcode-rel");
    const env = { [CATCODE_BIN_ENV]: "catcode-rel" } as Record<string, string | undefined>;
    expect(resolveCatcodeBinary(env, "linux", tmp)).toBe(abs);
  });

  test("returns null when nothing is found", () => {
    expect(resolveCatcodeBinary({ PATH: tmp } as Record<string, string | undefined>, "linux", tmp)).toBeNull();
    // The error text guides the user toward the fix.
    expect(CATCODE_NOT_FOUND_ERROR).toContain(CATCODE_BIN_ENV);
  });
});
