import { describe, expect, test } from "bun:test";
import {
  terminalOpenEnvelope,
  terminalSessionKey,
  terminalTerminateEnvelope,
} from "./terminal-protocol";

describe("terminal protocol identity", () => {
  test("same session ID in different workspaces has a different key", () => {
    expect(terminalSessionKey("owner", "/project-a", "term_1")).not.toBe(
      terminalSessionKey("owner", "/project-b", "term_1"),
    );
  });

  test("key encoding cannot collide when values contain separators", () => {
    expect(terminalSessionKey("a:b", "c", "d")).not.toBe(
      terminalSessionKey("a", "b:c", "d"),
    );
  });

  test("terminate envelopes always carry the workspace", () => {
    expect(terminalTerminateEnvelope("term_1", "/project-b")).toEqual({
      type: "terminate",
      sessionId: "term_1",
      workspace: "/project-b",
    });
  });

  test("reattach envelopes explicitly prevent replacement PTY creation", () => {
    const message = terminalOpenEnvelope("term_1", "/project", "", 80, 24, true);
    expect(message.attachOnly).toBe(true);
    expect(message.workspace).toBe("/project");
  });
});
