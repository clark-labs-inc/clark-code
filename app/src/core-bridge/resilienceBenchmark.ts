import contractJson from "./resilienceBenchmark.json";
import type {
  ProviderIncident,
  ProviderIncidentCategory,
  ProviderIncidentScope,
  Snapshot,
} from "./types";

export type ResilienceFault =
  | "rate_limit"
  | "duplicated_tool_ids"
  | "event_stream_disconnect"
  | "provider_process_loss"
  | "cloud_sync_delay"
  | "user_cancel";

interface ResilienceBenchmarkContract {
  version: number;
  storageKey: string;
  model: string;
  modelLabel: string;
  provider: "managed-agent";
  faults: ResilienceFault[];
}

export const RESILIENCE_BENCHMARK = contractJson as ResilienceBenchmarkContract;

export interface ResilienceCase {
  mask: number;
  id: string;
  faults: ResilienceFault[];
}

export interface StoredResilienceCase {
  version: number;
  mask: number;
  delayMs?: number;
}

export function resilienceCases(): ResilienceCase[] {
  const count = 2 ** RESILIENCE_BENCHMARK.faults.length;
  return Array.from({ length: count }, (_, mask) => {
    const faults = RESILIENCE_BENCHMARK.faults.filter((_, index) => (mask & (1 << index)) !== 0);
    return {
      mask,
      id: mask.toString(2).padStart(RESILIENCE_BENCHMARK.faults.length, "0"),
      faults,
    };
  });
}

export function parseStoredResilienceCase(raw: string | null): StoredResilienceCase | null {
  if (!raw) return null;
  try {
    const value = JSON.parse(raw) as Partial<StoredResilienceCase>;
    const maxMask = 2 ** RESILIENCE_BENCHMARK.faults.length;
    if (
      value.version !== RESILIENCE_BENCHMARK.version
      || !Number.isInteger(value.mask)
      || (value.mask ?? -1) < 0
      || (value.mask ?? maxMask) >= maxMask
      || (value.delayMs !== undefined && (!Number.isFinite(value.delayMs) || value.delayMs < 0))
    ) {
      return null;
    }
    return {
      version: value.version,
      mask: value.mask!,
      ...(value.delayMs === undefined ? {} : { delayMs: value.delayMs }),
    };
  } catch {
    return null;
  }
}

export function loadStoredResilienceCase(): StoredResilienceCase | null {
  try {
    return parseStoredResilienceCase(localStorage.getItem(RESILIENCE_BENCHMARK.storageKey));
  } catch {
    return null;
  }
}

export function faultsForMask(mask: number): Set<ResilienceFault> {
  return new Set(
    RESILIENCE_BENCHMARK.faults.filter((_, index) => (mask & (1 << index)) !== 0),
  );
}

interface SimulationHost {
  snapshot: Snapshot;
  run: string;
  emit: () => void;
  isCancelled: () => boolean;
  sleep: (ms: number) => Promise<void>;
  cancelWaitMs?: number;
}

function incident(
  run: string,
  suffix: string,
  now: number,
  scope: ProviderIncidentScope,
  category: ProviderIncidentCategory,
  message: string,
  detail: string,
  maxAttempts: number,
): ProviderIncident {
  return {
    id: `${run}-${suffix}`,
    status: "retrying",
    scope,
    failure_class: category === "rate_limit" ? "rate_limited" : "transient_transport",
    category,
    message,
    detail,
    model: RESILIENCE_BENCHMARK.model,
    provider_route: "http://127.0.0.1:11434/v1",
    ...(category === "rate_limit" ? { provider_status: 429, provider_error_type: "rate_limit" } : {}),
    request: {
      idempotency_key: `${run}-${suffix}-request`,
      provider_request_id: `benchmark-${suffix}`,
      attempts: 1,
      max_attempts: maxAttempts,
      retries: {
        transient: category === "rate_limit" ? 0 : 1,
        rate_limit: category === "rate_limit" ? 1 : 0,
        authentication: 0,
      },
      output_started: false,
      started_at_ms: now,
    },
    observed_at_ms: now,
    updated_at_ms: now,
  };
}

function appendIncident(snapshot: Snapshot, run: string, value: ProviderIncident) {
  snapshot.provider_incidents[value.id] = value;
  snapshot.timeline.push({ item: "provider_incident", run, id: value.id });
}

function settleActiveIncidents(snapshot: Snapshot, status: "recovered" | "interrupted", now: number) {
  for (const [id, value] of Object.entries(snapshot.provider_incidents)) {
    if (value.status !== "observed" && value.status !== "retrying") continue;
    snapshot.provider_incidents[id] = {
      ...value,
      status,
      updated_at_ms: now,
      ...(status === "recovered" ? { completed_at_ms: now } : {}),
    };
  }
}

/**
 * Deterministic browser-only fault injection at the same typed Snapshot seam
 * used by the native provider. It never pretends to be a paid model result:
 * the Playwright report labels every one of these cases `simulated`, and a
 * separate devbridge control exercises the real the agent-managed model route.
 */
export async function playResilienceSimulation(
  stored: StoredResilienceCase,
  host: SimulationHost,
): Promise<void> {
  const faults = faultsForMask(stored.mask);
  const delay = stored.delayMs ?? 35;
  const { snapshot, run } = host;
  const started = Date.now();

  snapshot.timeline.push({
    item: "message",
    run,
    role: "agent",
    phase: "commentary",
    blocks: [{
      type: "text",
      text: "I’m checking the saved execution boundary and recovery path before continuing.",
    }],
  });
  snapshot.execution_checklist = {
    revision: 1,
    steps: [
      { title: "Verify saved progress", status: "completed" },
      { title: "Recover the active request", status: "in_progress" },
      { title: "Confirm the conversation remains usable", status: "pending" },
    ],
  };
  snapshot.timeline.push({
    item: "execution_checklist",
    run,
    checklist: structuredClone(snapshot.execution_checklist),
  });

  snapshot.tool_calls["shell:89"] = {
    id: "shell:89",
    tool_name: "shell",
    title: "Inspect recovery fixture",
    kind: "execute",
    status: "completed",
    locations: [{ path: "README.md", line: 1 }],
    content: [{ type: "text", text: "Recovery fixture is readable." }],
  };
  snapshot.timeline.push({ item: "tool_call", id: "shell:89", run });

  if (faults.has("duplicated_tool_ids")) {
    snapshot.tool_calls.agent_loop_call_1 = {
      id: "agent_loop_call_1",
      tool_name: "shell",
      title: "Verify normalized tool result",
      kind: "execute",
      status: "completed",
      locations: [{ path: "README.md", line: 1 }],
      content: [{ type: "text", text: "The repeated provider identifier was normalized safely." }],
    };
    snapshot.timeline.push({ item: "tool_call", id: "agent_loop_call_1", run });
  }

  if (faults.has("rate_limit")) {
    appendIncident(snapshot, run, incident(
      run,
      "rate-limit",
      started,
      "model_request",
      "rate_limit",
      "the agent is waiting for temporary model capacity.",
      "The the agent-managed model route returned HTTP 429 before output began.",
      13,
    ));
  }
  if (faults.has("event_stream_disconnect")) {
    appendIncident(snapshot, run, incident(
      run,
      "event-stream",
      started,
      "provider_event_stream",
      "connection_lost",
      "the agent briefly lost the model connection.",
      "The response stream ended before any model output was committed.",
      4,
    ));
  }
  if (faults.has("cloud_sync_delay")) {
    snapshot.sync_pending = true;
    appendIncident(snapshot, run, incident(
      run,
      "cloud-sync",
      started,
      "cloud_history_sync",
      "connection_lost",
      "Cloud history sync is catching up.",
      "The durable local outbox is waiting for product cloud acknowledgment.",
      8,
    ));
  }
  if (faults.has("provider_process_loss")) {
    const processIncident = incident(
      run,
      "provider-process",
      started,
      "provider_process",
      "connection_lost",
      "the agent stopped unexpectedly. Your saved progress is intact.",
      "The local provider process ended after the transcript boundary was committed.",
      2,
    );
    processIncident.execution_recovery = {
      attempt: 1,
      started_at_ms: started,
      boundary: {
        execution_id: `${run}-execution`,
        attempt_sequence: 1,
        event_sequence: snapshot.timeline.length,
        transcript_commit_id: `${run}-commit`,
        completed_tools: faults.has("duplicated_tool_ids") ? 2 : 1,
        last_completed_tool_id: faults.has("duplicated_tool_ids")
          ? "agent_loop_call_1"
          : "shell:89",
        last_completed_tool_name: "shell",
        baseline_checkpoint_id: "benchmark-checkpoint",
      },
    };
    appendIncident(snapshot, run, processIncident);
  }
  host.emit();
  await host.sleep(delay);

  if (faults.has("user_cancel")) {
    const deadline = Date.now() + (host.cancelWaitMs ?? 4_000);
    while (!host.isCancelled() && Date.now() < deadline) {
      await host.sleep(Math.max(5, delay));
    }
    if (!host.isCancelled()) {
      snapshot.runs[run] = {
        id: run,
        status: "failed",
        outcome: {
          status: "failed",
          failure_kind: "local_state",
          error: "The benchmark did not observe the explicit cancellation signal.",
        },
        checkpoint: "benchmark-checkpoint",
      };
      host.emit();
      return;
    }
    const stopped = Date.now();
    settleActiveIncidents(snapshot, "interrupted", stopped);
    snapshot.sync_pending = faults.has("cloud_sync_delay") || undefined;
    snapshot.runs[run] = {
      id: run,
      status: "cancelled",
      outcome: { status: "cancelled", stop_reason: "user_cancelled" },
      checkpoint: "benchmark-checkpoint",
    };
    host.emit();
    return;
  }

  if (faults.has("provider_process_loss")) {
    const stopped = Date.now();
    for (const [id, value] of Object.entries(snapshot.provider_incidents)) {
      snapshot.provider_incidents[id] = {
        ...value,
        status: id.endsWith("provider-process") ? "interrupted" : "recovered",
        updated_at_ms: stopped,
        ...(id.endsWith("provider-process") ? {} : { completed_at_ms: stopped }),
      };
    }
    snapshot.sync_pending = faults.has("cloud_sync_delay") || undefined;
    snapshot.runs[run] = {
      id: run,
      status: "failed",
      outcome: {
        status: "failed",
        failure_kind: "runtime_interrupted",
        error: "the agent stopped before recovery completed.",
      },
      checkpoint: "benchmark-checkpoint",
    };
    host.emit();
    return;
  }

  await host.sleep(delay);
  settleActiveIncidents(snapshot, "recovered", Date.now());
  snapshot.sync_pending = undefined;
  snapshot.execution_checklist = {
    revision: 2,
    steps: [
      { title: "Verify saved progress", status: "completed" },
      { title: "Recover the active request", status: "completed" },
      { title: "Confirm the conversation remains usable", status: "completed" },
    ],
  };
  const checklist = snapshot.timeline.find(
    (item) => item.item === "execution_checklist" && item.run === run,
  );
  if (checklist?.item === "execution_checklist") {
    checklist.checklist = structuredClone(snapshot.execution_checklist);
  }
  snapshot.timeline.push({
    item: "message",
    run,
    role: "agent",
    phase: "final_answer",
    blocks: [{
      type: "text",
      text: "Recovery verified. The conversation stayed usable and saved progress remained intact. BENCHMARK_OK",
    }],
  });
  snapshot.runs[run] = {
    id: run,
    status: "done",
    outcome: { status: "done", stop_reason: "end_turn" },
    checkpoint: "benchmark-checkpoint",
  };
  host.emit();
}
