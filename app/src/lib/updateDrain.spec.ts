import { describe, expect, it } from "vitest";
import { emptySnapshot } from "../core-bridge/types";
import { sessionBlocksUpdate, updateDrainBlockerCount } from "./updateDrain";

function session(overrides: Partial<Parameters<typeof sessionBlocksUpdate>[0]> = {}) {
  return {
    live: emptySnapshot(),
    queuedCount: 0,
    dispatching: false,
    starting: false,
    ...overrides,
  };
}

describe("update drain", () => {
  it("waits for active runs, queued follow-ups, and prompt-start races", () => {
    const running = emptySnapshot();
    running.runs["run-1"] = { id: "run-1", status: "running" };

    expect(sessionBlocksUpdate(session({ live: running }))).toBe(true);
    expect(sessionBlocksUpdate(session({ queuedCount: 1 }))).toBe(true);
    expect(sessionBlocksUpdate(session({ dispatching: true }))).toBe(true);
    expect(sessionBlocksUpdate(session({ starting: true }))).toBe(true);
  });

  it("treats permission waits as active and counts blocked conversations", () => {
    const gated = emptySnapshot();
    gated.pending_permission = {
      id: "permission-1",
      session: "session-1",
      title: "Allow command?",
      options: [],
    };

    expect(
      updateDrainBlockerCount([
        session(),
        session({ live: gated }),
        session({ queuedCount: 2 }),
      ]),
    ).toBe(2);
  });
});
