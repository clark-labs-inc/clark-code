import { describe, expect, it } from "vitest";
import type { Snapshot } from "../core-bridge/types";
import { snapshotCheckpointIds } from "./checkpointRefs";

describe("snapshotCheckpointIds", () => {
  it("collects unique owned checkpoint refs and ignores runs without one", () => {
    const snapshot: Snapshot = {
      runs: {
        one: { id: "one", status: "done", checkpoint: "abc" },
        two: { id: "two", status: "done", checkpoint: "abc" },
        three: { id: "three", status: "done", checkpoint: "def" },
        four: { id: "four", status: "failed" },
      },
      timeline: [],
      tool_calls: {},
      artifacts: [],
      provider_incidents: {},
    };

    expect(snapshotCheckpointIds(snapshot)).toEqual(["abc", "def"]);
  });
});
