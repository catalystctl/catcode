import { describe, expect, test } from "bun:test";
import {
  nodeVersionIsTooOld,
  runtimeRequirementError,
} from "./runtime-requirements.mjs";

describe("web runtime requirements", () => {
  test("accepts Node 22.13 and newer major versions", () => {
    expect(nodeVersionIsTooOld("22.13.0")).toBe(false);
    expect(nodeVersionIsTooOld("22.21.1")).toBe(false);
    expect(nodeVersionIsTooOld("23.0.0")).toBe(false);
  });

  test("rejects older and malformed Node versions", () => {
    expect(nodeVersionIsTooOld("22.12.9")).toBe(true);
    expect(nodeVersionIsTooOld("21.99.0")).toBe(true);
    expect(nodeVersionIsTooOld("not-a-version")).toBe(true);
  });

  test("rejects Bun's node shim with an actionable explanation", () => {
    expect(runtimeRequirementError({ node: "24.0.0", bun: "1.3.14" })).toContain(
      "Bun's node shim is not supported",
    );
  });
});
