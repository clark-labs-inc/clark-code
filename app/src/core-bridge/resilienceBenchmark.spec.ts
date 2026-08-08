import { describe, expect, it } from "vitest";

import {
  faultsForMask,
  parseStoredResilienceCase,
  playResilienceSimulation,
  RESILIENCE_BENCHMARK,
  resilienceCases,
  type ResilienceFault,
} from "./resilienceBenchmark";
import { emptySnapshot, type Snapshot } from "./types";

function snapshotFor(run: string): Snapshot {
  const snapshot = emptySnapshot();
  snapshot.session = "benchmark-session";
  snapshot.runs[run] = { id: run, status: "running" };
  return snapshot;
}

async function simulate(mask: number) {
  const run = `run-${mask}`;
  const snapshot = snapshotFor(run);
  let emissions = 0;
  await playResilienceSimulation(
    { version: RESILIENCE_BENCHMARK.version, mask, delayMs: 0 },
    {
      snapshot,
      run,
      emit: () => { emissions += 1; },
      isCancelled: () => faultsForMask(mask).has("user_cancel"),
      sleep: async () => {},
      cancelWaitMs: 0,
    },
  );
  return { snapshot, emissions, run };
}

describe("resilience benchmark contract", () => {
  it("enumerates the complete six-dimensional power set", () => {
    const cases = resilienceCases();
    expect(cases).toHaveLength(64);
    expect(new Set(cases.map((value) => value.id))).toHaveLength(64);

    for (const fault of RESILIENCE_BENCHMARK.faults) {
      expect(cases.filter((value) => value.faults.includes(fault))).toHaveLength(32);
    }
    for (let left = 0; left < RESILIENCE_BENCHMARK.faults.length; left += 1) {
      for (let right = left + 1; right < RESILIENCE_BENCHMARK.faults.length; right += 1) {
        expect(cases.filter((value) =>
          value.faults.includes(RESILIENCE_BENCHMARK.faults[left])
          && value.faults.includes(RESILIENCE_BENCHMARK.faults[right]),
        )).toHaveLength(16);
      }
    }
  });

  it("accepts only the versioned in-range storage envelope", () => {
    expect(parseStoredResilienceCase(JSON.stringify({ version: 1, mask: 63, delayMs: 0 })))
      .toEqual({ version: 1, mask: 63, delayMs: 0 });
    expect(parseStoredResilienceCase(JSON.stringify({ version: 2, mask: 0 }))).toBeNull();
    expect(parseStoredResilienceCase(JSON.stringify({ version: 1, mask: 64 }))).toBeNull();
    expect(parseStoredResilienceCase(JSON.stringify({ version: 1, mask: -1 }))).toBeNull();
    expect(parseStoredResilienceCase("not-json")).toBeNull();
  });

  it("settles every combination without duplicate canonical tool ids", async () => {
    for (const testCase of resilienceCases()) {
      const { snapshot, emissions, run } = await simulate(testCase.mask);
      const faults = new Set<ResilienceFault>(testCase.faults);
      const expectedStatus = faults.has("user_cancel")
        ? "cancelled"
        : faults.has("provider_process_loss")
          ? "failed"
          : "done";
      expect(snapshot.runs[run].status, testCase.id).toBe(expectedStatus);
      expect(emissions, testCase.id).toBeGreaterThanOrEqual(2);

      const toolIds = Object.keys(snapshot.tool_calls);
      expect(new Set(toolIds).size, testCase.id).toBe(toolIds.length);
      expect(toolIds, testCase.id).toHaveLength(faults.has("duplicated_tool_ids") ? 2 : 1);
      if (faults.has("duplicated_tool_ids")) {
        expect(toolIds).toEqual(["shell:89", "agent_loop_call_1"]);
      }

      const expectedIncidents = [
        "rate_limit",
        "event_stream_disconnect",
        "provider_process_loss",
        "cloud_sync_delay",
      ].filter((fault) => faults.has(fault as ResilienceFault)).length;
      expect(Object.keys(snapshot.provider_incidents), testCase.id).toHaveLength(expectedIncidents);
      expect(Object.values(snapshot.provider_incidents).every(
        (value) => value.status !== "retrying" && value.status !== "observed",
      ), testCase.id).toBe(true);
    }
  });

  it("keeps a delayed cloud write pending after interruption and clears it after recovery", async () => {
    const cloudIndex = RESILIENCE_BENCHMARK.faults.indexOf("cloud_sync_delay");
    const processIndex = RESILIENCE_BENCHMARK.faults.indexOf("provider_process_loss");
    const interrupted = await simulate((1 << cloudIndex) | (1 << processIndex));
    expect(interrupted.snapshot.sync_pending).toBe(true);
    expect(interrupted.snapshot.runs[interrupted.run].outcome?.failure_kind)
      .toBe("runtime_interrupted");

    const recovered = await simulate(1 << cloudIndex);
    expect(recovered.snapshot.sync_pending).toBeUndefined();
    expect(Object.values(recovered.snapshot.provider_incidents)[0].status).toBe("recovered");
  });

  it("fails the cancellation case unless the UI delivers an explicit stop signal", async () => {
    const cancelIndex = RESILIENCE_BENCHMARK.faults.indexOf("user_cancel");
    const mask = 1 << cancelIndex;
    const run = "missing-cancel";
    const snapshot = snapshotFor(run);

    await playResilienceSimulation(
      { version: RESILIENCE_BENCHMARK.version, mask, delayMs: 0 },
      {
        snapshot,
        run,
        emit: () => {},
        isCancelled: () => false,
        sleep: async () => {},
        cancelWaitMs: 0,
      },
    );

    expect(snapshot.runs[run]).toEqual(expect.objectContaining({
      status: "failed",
      outcome: expect.objectContaining({
        failure_kind: "local_state",
        error: "The benchmark did not observe the explicit cancellation signal.",
      }),
    }));
  });
});
