import { describe, expect, it } from "vitest";
import { emptySnapshot, type Snapshot } from "../core-bridge/types";
import { deferredSnapshotForPersistence } from "./snapshotPersistence";

describe("deferredSnapshotForPersistence", () => {
  it("accepts the live snapshot generation", () => {
    const running = { ...emptySnapshot(), session: "conversation-1" };

    expect(deferredSnapshotForPersistence(running, running, true, true)).toBe(running);
  });

  it("rejects a running snapshot after a terminal projection arrived", () => {
    const running = { ...emptySnapshot(), session: "conversation-1" };
    const terminal = { ...running, runs: {} };

    expect(deferredSnapshotForPersistence(terminal, running, true, false)).toBeNull();
  });

  it("reproduces and fences the deferred-running-after-terminal ordering", () => {
    const running = { ...emptySnapshot(), session: "conversation-1" };
    const terminal = { ...running, runs: {} };
    let latest = running;
    const persisted: Snapshot[] = [];
    const deferredRunningPersist = () => {
      const snapshot = deferredSnapshotForPersistence(latest, running, true, false);
      if (snapshot) persisted.push(snapshot);
    };

    latest = terminal;
    persisted.push(terminal);
    deferredRunningPersist();

    expect(persisted).toEqual([terminal]);
  });
  it("keeps the first publication immediate by selecting the newest active delta instead of dropping the checkpoint", () => {
    const first = { ...emptySnapshot(), session: "conversation-1" };
    const latest = { ...first, starting: true };
    expect(deferredSnapshotForPersistence(latest, first, true, true)).toBe(latest);
    expect(deferredSnapshotForPersistence({ ...latest, session: "other" }, first, true, true)).toBeNull();
  });
});
