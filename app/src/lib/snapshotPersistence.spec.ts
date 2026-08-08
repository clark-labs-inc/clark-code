import { describe, expect, it } from "vitest";
import { emptySnapshot } from "../core-bridge/types";
import { deferredSnapshotPersistIsCurrent } from "./snapshotPersistence";

describe("deferredSnapshotPersistIsCurrent", () => {
  it("accepts the live snapshot generation", () => {
    const running = { ...emptySnapshot(), session: "conversation-1" };

    expect(deferredSnapshotPersistIsCurrent(running, running)).toBe(true);
  });

  it("rejects a running snapshot after a terminal projection arrived", () => {
    const running = { ...emptySnapshot(), session: "conversation-1" };
    const terminal = { ...running, runs: {} };

    expect(deferredSnapshotPersistIsCurrent(terminal, running)).toBe(false);
  });

  it("reproduces and fences the deferred-running-after-terminal ordering", () => {
    const running = { ...emptySnapshot(), session: "conversation-1" };
    const terminal = { ...running, runs: {} };
    let latest = running;
    const persisted: typeof running[] = [];
    const deferredRunningPersist = () => {
      if (deferredSnapshotPersistIsCurrent(latest, running)) persisted.push(running);
    };

    latest = terminal;
    persisted.push(terminal);
    deferredRunningPersist();

    expect(persisted).toEqual([terminal]);
  });
});
