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

const SCENARIO_IDS = [
  "clark-code-rate-limit-recovery",
  "clark-code-request-timeout-recovery",
  "clark-code-upstream-unavailable-recovery",
  "clark-code-duplicate-tool-id-normalization",
  "clark-code-event-stream-reconnect",
  "clark-code-tool-host-reconnect",
  "clark-code-provider-process-loss-checkpoint",
  "clark-code-cloud-sync-recovery",
  "clark-code-explicit-user-cancel",
  "clark-code-all-recoverable-faults",
] as const;
const FAULT_SCENARIO_IDS: Record<ResilienceFault, (typeof SCENARIO_IDS)[number]> = {
  rate_limit: "clark-code-rate-limit-recovery",
  request_timeout: "clark-code-request-timeout-recovery",
  upstream_unavailable: "clark-code-upstream-unavailable-recovery",
  duplicated_tool_ids: "clark-code-duplicate-tool-id-normalization",
  event_stream_disconnect: "clark-code-event-stream-reconnect",
  tool_host_disconnect: "clark-code-tool-host-reconnect",
  provider_process_loss: "clark-code-provider-process-loss-checkpoint",
  cloud_sync_delay: "clark-code-cloud-sync-recovery",
  user_cancel: "clark-code-explicit-user-cancel",
};

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
  it("keeps every CI scenario id bound to the versioned fault contract", () => {
    expect(SCENARIO_IDS).toHaveLength(10);
    expect(new Set(SCENARIO_IDS).size).toBe(SCENARIO_IDS.length);
    expect(Object.keys(FAULT_SCENARIO_IDS)).toEqual(RESILIENCE_BENCHMARK.faults);
    expect(Object.values(FAULT_SCENARIO_IDS).every((id) => SCENARIO_IDS.includes(id)))
      .toBe(true);
  });

  it("enumerates the complete fault power set", () => {
    const cases = resilienceCases();
    const expectedCaseCount = 2 ** RESILIENCE_BENCHMARK.faults.length;
    expect(cases).toHaveLength(expectedCaseCount);
    expect(new Set(cases.map((value) => value.id))).toHaveLength(expectedCaseCount);

    for (const fault of RESILIENCE_BENCHMARK.faults) {
      expect(cases.filter((value) => value.faults.includes(fault)))
        .toHaveLength(expectedCaseCount / 2);
    }
    for (let left = 0; left < RESILIENCE_BENCHMARK.faults.length; left += 1) {
      for (let right = left + 1; right < RESILIENCE_BENCHMARK.faults.length; right += 1) {
        expect(cases.filter((value) =>
          value.faults.includes(RESILIENCE_BENCHMARK.faults[left])
          && value.faults.includes(RESILIENCE_BENCHMARK.faults[right]),
        )).toHaveLength(expectedCaseCount / 4);
      }
    }
  });

  it("accepts only the versioned in-range storage envelope", () => {
    const maxMask = 2 ** RESILIENCE_BENCHMARK.faults.length;
    const version = RESILIENCE_BENCHMARK.version;
    expect(parseStoredResilienceCase(JSON.stringify({ version, mask: maxMask - 1, delayMs: 0 })))
      .toEqual({ version, mask: maxMask - 1, delayMs: 0 });
    expect(parseStoredResilienceCase(JSON.stringify({ version: version + 1, mask: 0 }))).toBeNull();
    expect(parseStoredResilienceCase(JSON.stringify({ version, mask: maxMask }))).toBeNull();
    expect(parseStoredResilienceCase(JSON.stringify({ version, mask: -1 }))).toBeNull();
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
        "request_timeout",
        "upstream_unavailable",
        "event_stream_disconnect",
        "tool_host_disconnect",
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

  it.each([
    ["request_timeout", "timeout", "model_request", 504],
    ["upstream_unavailable", "upstream_unavailable", "model_request", 503],
    ["tool_host_disconnect", "connection_lost", "tool_execution_host", undefined],
  ] as const)(
    "projects the %s fault through exact incident diagnostics before recovery",
    async (fault, category, scope, providerStatus) => {
      const index = RESILIENCE_BENCHMARK.faults.indexOf(fault);
      const run = `typed-${fault}`;
      const snapshot = snapshotFor(run);
      const emissions: Snapshot[] = [];

      await playResilienceSimulation(
        { version: RESILIENCE_BENCHMARK.version, mask: 1 << index, delayMs: 0 },
        {
          snapshot,
          run,
          emit: () => emissions.push(structuredClone(snapshot)),
          isCancelled: () => false,
          sleep: async () => {},
        },
      );

      const activeIncident = emissions
        .flatMap((value) => Object.values(value.provider_incidents))
        .find((value) => value.status === "retrying");
      expect(activeIncident).toMatchObject({
        category,
        scope,
        request: {
          attempts: 1,
          output_started: false,
        },
        ...(providerStatus === undefined ? {} : { provider_status: providerStatus }),
      });
      if (providerStatus === undefined) {
        expect(activeIncident).not.toHaveProperty("provider_status");
      }
      expect(Object.values(snapshot.provider_incidents)).toEqual([
        expect.objectContaining({ status: "recovered", category, scope }),
      ]);
      expect(snapshot.runs[run].status).toBe("done");
    },
  );

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
