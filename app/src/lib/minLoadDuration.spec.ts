import { describe, it, expect, vi } from "vitest";
import { minLoadDuration } from "./minLoadDuration";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe("minLoadDuration", () => {
  it("holds the loading flag for at least the floor when work resolves instantly", async () => {
    vi.useFakeTimers();
    let settled = false;
    const p = minLoadDuration(Promise.resolve("ok"), 250).then(() => {
      settled = true;
    });
    // Before the floor elapses: not settled yet.
    await vi.advanceTimersByTimeAsync(249);
    expect(settled).toBe(false);
    // At the floor: settles.
    await vi.advanceTimersByTimeAsync(2);
    vi.useRealTimers();
    await p;
    expect(settled).toBe(true);
  });

  it("returns work's value after the floor", async () => {
    const value = await minLoadDuration(Promise.resolve(42), 10);
    expect(value).toBe(42);
  });

  it("re-throws work's rejection, but not before the floor elapses", async () => {
    vi.useFakeTimers();
    let rejected = false;
    const p = minLoadDuration(Promise.reject(new Error("boom")), 250).then(
      () => "no",
      () => {
        rejected = true;
      },
    );
    // Before the floor elapses: the instant rejection has not surfaced yet.
    await vi.advanceTimersByTimeAsync(249);
    expect(rejected).toBe(false);
    // After the floor: the rejection surfaces.
    await vi.advanceTimersByTimeAsync(2);
    vi.useRealTimers();
    await p;
    expect(rejected).toBe(true);
  });

  it("never waits longer than the work itself", async () => {
    const start = performance.now();
    await minLoadDuration(sleep(300), 10); // work slower than floor
    const elapsed = performance.now() - start;
    expect(elapsed).toBeGreaterThanOrEqual(290);
    expect(elapsed).toBeLessThan(60 + 300); // not the floor + work, just max(floor, work)
  });
});
