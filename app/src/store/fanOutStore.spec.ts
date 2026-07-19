import { afterEach, describe, expect, it } from "vitest";
import type { FanOut } from "../core-bridge/types";
import { resetFanOut, syncFanOut, useFanOutStore } from "./fanOutStore";

const fanOut: FanOut = {
  title: "Parallel work",
  total: 2,
  done: 0,
  running: 1,
  agents: [
    { id: "queued", label: "Queued task", status: "queued" },
    { id: "running", label: "Running task", status: "running", activity: "Reading files" },
  ],
};

afterEach(() => resetFanOut());

describe("fanOutStore", () => {
  it("selects the running child by default and opens a requested child", () => {
    syncFanOut(fanOut);
    expect(useFanOutStore.getState().selectedAgentId).toBe("running");

    useFanOutStore.getState().openInspector("queued");
    expect(useFanOutStore.getState().inspectorOpen).toBe(true);
    expect(useFanOutStore.getState().selectedAgentId).toBe("queued");
  });

  it("syncs public activity updates even when status is unchanged", () => {
    syncFanOut(fanOut);
    syncFanOut({
      ...fanOut,
      agents: fanOut.agents.map((agent) =>
        agent.id === "running" ? { ...agent, activity: "Checking tests" } : agent,
      ),
    });

    expect(
      useFanOutStore.getState().fanOut?.agents.find((agent) => agent.id === "running")?.activity,
    ).toBe("Checking tests");
  });
});
