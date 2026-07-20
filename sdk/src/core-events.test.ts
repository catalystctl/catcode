import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { CORE_EVENT_TYPES, isKnownCoreEventType } from "./core-events.js";

describe("SDK protocol compatibility", () => {
  test("accepts the checked-in v2 hello fixture", () => {
    const fixture = JSON.parse(
      readFileSync(new URL("../../protocol/fixtures/protocol-hello-v2.json", import.meta.url), "utf8"),
    );
    expect(fixture.type).toBe("protocol_hello");
    expect(fixture.protocol_version).toBe(2);
    expect(Array.isArray(fixture.capabilities)).toBe(true);
    expect(isKnownCoreEventType(fixture.type)).toBe(true);
  });

  test("catalog is unique and unknown future events remain tolerated", () => {
    expect(new Set(CORE_EVENT_TYPES).size).toBe(CORE_EVENT_TYPES.length);
    expect(isKnownCoreEventType("future_optional_event")).toBe(false);
    const futureEvent: { type: string; [key: string]: unknown } = {
      type: "future_optional_event",
      optional: true,
    };
    expect(futureEvent.optional).toBe(true);
  });

  test("accepts an exhaustive checked-in event catalog fixture", () => {
    const fixtures = readFileSync(
      new URL("../../protocol/fixtures/events-v2.jsonl", import.meta.url),
      "utf8",
    )
      .trim()
      .split(/\r?\n/)
      .map((line) => JSON.parse(line));
    expect(fixtures.map((event) => event.type)).toEqual([...CORE_EVENT_TYPES]);
    expect(fixtures.every((event) => event.protocol_version === 2)).toBe(true);
  });
});
