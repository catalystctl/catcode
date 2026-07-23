import { describe, expect, test } from "bun:test";
import { reconcileSavedSnapshot, SerializedTaskQueue } from "./editor-save-state";

function deferred() {
  let resolve!: () => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<void>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

describe("editor save state", () => {
  test("keeps edits made after request submission dirty", () => {
    expect(reconcileSavedSnapshot("submitted", "submitted-later edit")).toEqual({
      saved: "submitted",
      dirty: true,
    });
    expect(reconcileSavedSnapshot("submitted", "submitted")).toEqual({
      saved: "submitted",
      dirty: false,
    });
  });

  test("serializes overlapping saves in submission order", async () => {
    const first = deferred();
    const second = deferred();
    const calls: string[] = [];
    const queue = new SerializedTaskQueue();

    const a = queue.enqueue(async () => {
      calls.push("first:start");
      await first.promise;
      calls.push("first:end");
    });
    const b = queue.enqueue(async () => {
      calls.push("second:start");
      await second.promise;
      calls.push("second:end");
    });

    await Bun.sleep(0);
    expect(calls).toEqual(["first:start"]);
    first.resolve();
    await a;
    await Bun.sleep(0);
    expect(calls).toEqual(["first:start", "first:end", "second:start"]);
    second.resolve();
    await b;
    expect(calls).toEqual(["first:start", "first:end", "second:start", "second:end"]);
  });

  test("continues after a failed save and reports pending work accurately", async () => {
    const pending: number[] = [];
    const queue = new SerializedTaskQueue((count) => pending.push(count));
    const failed = queue.enqueue(async () => {
      throw new Error("disk full");
    });
    const recovered = queue.enqueue(async () => {});

    await expect(failed).rejects.toThrow("disk full");
    await recovered;
    expect(pending).toEqual([1, 2, 1, 0]);
  });
});
