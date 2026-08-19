import { describe, expect, it } from "vitest";
import { emptySnapshot, type RunStatus } from "../core-bridge/types";
import { latestRunFailed, useSessionStore } from "./sessionStore";

function snapshotWithRuns(...runs: Array<[string, RunStatus]>) {
  const snapshot = emptySnapshot();
  snapshot.runs = Object.fromEntries(
    runs.map(([id, status]) => [id, { id, status }]),
  );
  return snapshot;
}

describe("run completion notifications", () => {
  it("keeps warnings out of the success notice channel", () => {
    const state = useSessionStore.getState();
    state.flashNotice("saved");
    state.flashWarning("could not load");

    expect(useSessionStore.getState()).toMatchObject({
      notice: "saved",
      warning: "could not load",
    });
    state.dismissNotice();
    state.dismissWarning();
  });

  it("does not let an earlier failed run poison a later successful completion", () => {
    const snapshot = snapshotWithRuns(
      ["failed-run", "failed"],
      ["successful-run", "done"],
    );

    expect(latestRunFailed(snapshot)).toBe(false);
  });

  it("reports a failure when the run that just settled failed", () => {
    const snapshot = snapshotWithRuns(
      ["successful-run", "done"],
      ["failed-run", "failed"],
    );

    expect(latestRunFailed(snapshot)).toBe(true);
  });
});
