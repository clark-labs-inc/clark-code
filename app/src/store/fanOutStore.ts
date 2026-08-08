import { create } from "zustand";
import type { FanOut, FanOutAgent } from "../core-bridge/types";

export type { FanOut, FanOutAgent };

interface FanOutState {
  fanOut: FanOut | null;
  inspectorOpen: boolean;
  selectedAgentId: string | null;
  setFanOut: (f: FanOut | null) => void;
  openInspector: (agentId?: string) => void;
  closeInspector: () => void;
  selectAgent: (agentId: string) => void;
  clearFanOut: () => void;
}

/** Kept as its own tiny store (not folded into sessionStore) so the fan-out
 *  surface is fully self-contained and cheap to subscribe to. Fed by
 *  `syncFanOut` from the session snapshot, which is the projection of
 *  per-child `subagent_event` telemetry (see agent-core `Snapshot::fan_out`). */
export const useFanOutStore = create<FanOutState>((set) => ({
  fanOut: null,
  inspectorOpen: false,
  selectedAgentId: null,
  setFanOut: (fanOut) =>
    set((state) => {
      if (!fanOut) {
        return { fanOut: null, inspectorOpen: false, selectedAgentId: null };
      }
      const selectedStillExists = fanOut.agents.some(
        (agent) => agent.id === state.selectedAgentId,
      );
      const selectedAgentId = selectedStillExists
        ? state.selectedAgentId
        : (fanOut.agents.find((agent) => agent.status === "running")?.id ??
          fanOut.agents[0]?.id ??
          null);
      return { fanOut, selectedAgentId };
    }),
  openInspector: (agentId) =>
    set((state) => ({
      inspectorOpen: state.fanOut !== null,
      selectedAgentId:
        agentId && state.fanOut?.agents.some((agent) => agent.id === agentId)
          ? agentId
          : state.selectedAgentId,
    })),
  closeInspector: () => set({ inspectorOpen: false }),
  selectAgent: (selectedAgentId) => set({ selectedAgentId }),
  clearFanOut: () => set({ fanOut: null, inspectorOpen: false, selectedAgentId: null }),
}));

/** A cheap content signature so we only push (and re-render the panel) when the
 *  fan-out actually changed, not on every re-cloned snapshot during streaming. */
function signature(f: FanOut | null | undefined): string {
  if (!f) return "";
  return `${f.title}:${f.total}:${f.done}:${f.running}:${f.agents
    .map(
      (agent) =>
        `${agent.id}:${agent.label}:${agent.status}:${agent.objective ?? ""}:${agent.activity ?? ""}:${
          agent.result ?? ""
        }:${agent.attempt ?? ""}:${agent.started_at_ms ?? ""}:${agent.updated_at_ms ?? ""}`,
    )
    .join(",")}`;
}

let lastSignature = "";

/** Push the snapshot's fan-out into the store, deduped by content. Call this
 *  wherever the session snapshot is applied. */
export function syncFanOut(f: FanOut | null | undefined): void {
  const sig = signature(f);
  if (sig === lastSignature) return;
  lastSignature = sig;
  useFanOutStore.getState().setFanOut(f ?? null);
}

/** Clear the fan-out AND reset the dedup signature, so the next `syncFanOut`
 *  for a different conversation always re-pushes (a switch to an idle session
 *  emits no snapshot frame, so nothing else would clear a prior swarm). */
export function resetFanOut(): void {
  lastSignature = "";
  useFanOutStore.getState().clearFanOut();
}

/** Dev-only: preview the fan-out surface without a live run. `previewFanOut()`
 *  in the console. */
export function previewFanOut(): void {
  const now = Date.now();
  const agents: FanOutAgent[] = [
    {
      id: "platform-endpoint-survey",
      label: "Platform endpoint survey",
      status: "done",
      objective: "Trace the agent server route, auth, access, and artifact seams.",
      activity: "Complete",
      result: "Confirmed the platform route and authentication boundary.",
      attempt: 1,
      started_at_ms: now - 82_000,
      updated_at_ms: now - 21_000,
    },
    {
      id: "desktop-tool-wiring",
      label: "Desktop tool wiring",
      status: "running",
      objective: "Add typed local image tools without exposing provider credentials.",
      activity: "Reviewing the provider-local tool registry",
      attempt: 1,
      started_at_ms: now - 54_000,
      updated_at_ms: now,
    },
    {
      id: "image-workflow-verification",
      label: "Image workflow verification",
      status: "queued",
      objective: "Verify viewing, editing, and generated-image artifacts end to end.",
      activity: "Waiting to start",
      updated_at_ms: now,
    },
  ];
  lastSignature = "preview";
  useFanOutStore.getState().setFanOut({
    title: "Build the feature in isolated workspaces, then combine and verify the result",
    total: agents.length,
    done: 1,
    running: 1,
    agents,
  });
}

if (import.meta.env.DEV && typeof window !== "undefined") {
  (window as unknown as { previewFanOut?: () => void }).previewFanOut = previewFanOut;
}
