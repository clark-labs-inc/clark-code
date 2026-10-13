import { afterEach, describe, expect, it, vi } from "vitest";
import { emptySnapshot, type Snapshot } from "../core-bridge/types";
import { SnapshotPublicationCadence } from "./snapshotPublicationCadence";

function message(text: string, index = 0): Snapshot {
  return { ...emptySnapshot(), timeline: Array.from({ length: index + 1 }, (_, position) => ({
    item: "message", run: "run", role: "agent", blocks: [{ type: "text", text: position === index ? text : "Earlier reply" }],
  })) };
}
afterEach(() => vi.useRealTimers());

describe("streaming snapshot publication cadence", () => {
  it("publishes the first public token immediately after a just-published run start", () => {
    const cadence = new SnapshotPublicationCadence();
    expect(cadence.shouldPublish(emptySnapshot(), true, 0)).toBe(true);
    expect(cadence.shouldPublish(message("First"), true, 10)).toBe(true);
  });
  it("bounds subsequent growing text to one publication per 250ms", () => {
    const cadence = new SnapshotPublicationCadence();
    expect(cadence.shouldPublish(message("First"), true, 0)).toBe(true);
    expect(cadence.shouldPublish(message("First token"), true, 249)).toBe(false);
    expect(cadence.shouldPublish(message("First token arrived"), true, 250)).toBe(true);
  });
  it("flushes the next reply's first token even within the normal publication interval", () => {
    const cadence = new SnapshotPublicationCadence();
    cadence.shouldPublish(message("Earlier"), true, 0);
    expect(cadence.shouldPublish(message("Next", 1), true, 10)).toBe(true);
  });
  it("always publishes terminal state immediately", () => {
    const cadence = new SnapshotPublicationCadence();
    cadence.shouldPublish(message("Partial"), true, 0);
    expect(cadence.shouldPublish(message("Complete"), false, 1)).toBe(true);
  });
  it("flushes the newest held text at 250ms even when the provider pauses", () => {
    vi.useFakeTimers(); vi.setSystemTime(0);
    const cadence = new SnapshotPublicationCadence();
    const writes: string[] = [];
    cadence.schedule(message("First"), true, () => writes.push("First"));
    vi.advanceTimersByTime(10);
    cadence.schedule(message("First token"), true, () => writes.push("First token"));
    vi.advanceTimersByTime(10);
    cadence.schedule(message("First token received"), true, () => writes.push("First token received"));
    expect(writes).toEqual(["First"]);
    vi.advanceTimersByTime(230);
    expect(writes).toEqual(["First", "First token received"]);
  });
  it("cancels an older trailing callback when the run settles", () => {
    vi.useFakeTimers(); vi.setSystemTime(0);
    const cadence = new SnapshotPublicationCadence();
    const writes: string[] = [];
    cadence.schedule(message("First"), true, () => writes.push("First"));
    vi.advanceTimersByTime(10);
    cadence.schedule(message("Partial"), true, () => writes.push("Partial"));
    cadence.schedule(message("Done"), false, () => writes.push("Done"));
    vi.advanceTimersByTime(250);
    expect(writes).toEqual(["First", "Done"]);
  });
  it("does not bypass the cadence for private reasoning", () => {
    const cadence = new SnapshotPublicationCadence();
    cadence.shouldPublish(emptySnapshot(), true, 0);
    expect(cadence.shouldPublish(message("<thinking>private</thinking>"), true, 10)).toBe(false);
    expect(cadence.shouldPublish(message("<thinking>private</thinking>Visible"), true, 20)).toBe(true);
  });
});
