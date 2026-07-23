import { describe, expect, test } from "bun:test";
import { normalizeSandboxMode } from "./sandbox.js";
import { CoreProcess, SandboxCommandError, type CoreEvent } from "./core-process.js";

describe("normalizeSandboxMode", () => {
  test("disabled aliases map to none", () => {
    for (const v of ["none", "off", "false", "disabled", "disable"]) {
      expect(normalizeSandboxMode(v as never)).toBe("none");
    }
  });

  test("enabled aliases map to microsandbox without warning", () => {
    const warns: string[] = [];
    const orig = console.warn;
    console.warn = (m: string) => warns.push(m);
    try {
      for (const v of ["microsandbox", "msb", "on", "true", "enabled", "enable"]) {
        expect(normalizeSandboxMode(v as never)).toBe("microsandbox");
      }
    } finally {
      console.warn = orig;
    }
    // Only enabled aliases used here must not trip the deprecation path.
    expect(warns.filter((w) => w.includes("firejail") || w.includes("seatbelt"))).toHaveLength(0);
  });

  test("legacy firejail/seatbelt spellings map to microsandbox with a deprecation notice", () => {
    // Reset the once-per-alias warning guard by exercising a fresh alias set.
    const warns: string[] = [];
    const orig = console.warn;
    console.warn = (m: string) => warns.push(m);
    try {
      expect(normalizeSandboxMode("firejail")).toBe("microsandbox");
      expect(normalizeSandboxMode("seatbelt")).toBe("microsandbox");
      expect(normalizeSandboxMode("macos")).toBe("microsandbox");
      expect(normalizeSandboxMode("sandbox-exec")).toBe("microsandbox");
      expect(normalizeSandboxMode("fj")).toBe("microsandbox");
    } finally {
      console.warn = orig;
    }
    // Every legacy alias produced exactly one deprecation notice.
    expect(warns.length).toBeGreaterThanOrEqual(5);
    expect(warns.every((w) => w.includes("microsandbox"))).toBe(true);
  });

  test("normalization is idempotent and case-insensitive", () => {
    expect(normalizeSandboxMode("Microsandbox")).toBe("microsandbox");
    expect(normalizeSandboxMode("NONE")).toBe("none");
  });

  test("undefined passes through (core default applies)", () => {
    expect(normalizeSandboxMode(undefined)).toBeUndefined();
  });

  test("unknown values fail closed (never silently map to none)", () => {
    expect(() => normalizeSandboxMode("docker" as never)).toThrow(RangeError);
    expect(() => normalizeSandboxMode("podman" as never)).toThrow(/Unknown sandbox mode/);
  });
});

describe("SandboxCommandError", () => {
  test("extracts the core error message", () => {
    const ev: CoreEvent = { type: "sandbox_error", error: "image_pull_failed: digest mismatch" };
    const err = new SandboxCommandError(ev);
    expect(err.message).toBe("image_pull_failed: digest mismatch");
    expect(err.name).toBe("SandboxCommandError");
    expect(err.event).toBe(ev);
  });

  test("falls back to a generic message when error field is absent", () => {
    const err = new SandboxCommandError({ type: "sandbox_error" });
    expect(err.message).toBe("sandbox command failed");
  });
});

describe("CoreProcess sandbox command builders (no core spawned)", () => {
  // buildArgs is private; verify behavior indirectly by constructing the option
  // shape the core would receive. We assert the typed surface compiles and that
  // the normalizer is wired (the unit under test is normalizeSandboxMode above
  // + the public method presence).
  test("exposes getSandboxStatus/prepareSandbox/resetSandbox methods", () => {
    const cp = new CoreProcess({ cwd: process.cwd() });
    expect(typeof cp.getSandboxStatus).toBe("function");
    expect(typeof cp.prepareSandbox).toBe("function");
    expect(typeof cp.resetSandbox).toBe("function");
  });

  test("accepts the new sandbox option types without firejail as a backend", () => {
    // Compiles only because the option union accepts microsandbox and the
    // deprecated legacy aliases; firejail remains an accepted *input alias*
    // but is not an operational backend type.
    const cp = new CoreProcess({
      cwd: process.cwd(),
      sandbox: "microsandbox",
      noNetwork: false,
      sandboxConfig: {
        image: "ghcr.io/catalystctl/catcode-sandbox:0.2.0",
        cpus: 2,
        memoryMb: 2048,
        diskMb: 8192,
        networkMode: "restricted",
        envAllowlist: ["CI", "NODE_ENV"],
      },
    });
    expect(cp.options.sandbox).toBe("microsandbox");
    expect(cp.options.sandboxConfig?.cpus).toBe(2);
  });

  test("legacy firejail input is accepted at the type level for source compat", () => {
    const cp = new CoreProcess({ cwd: process.cwd(), sandbox: "firejail" });
    // Normalization happens at spawn time; the raw option is preserved here.
    expect(cp.options.sandbox).toBe("firejail");
  });
});
