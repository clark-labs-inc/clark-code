import { create } from "zustand";
import type { FanOut, FanOutAgent } from "../core-bridge/types";

export type { FanOut, FanOutAgent };

interface FanOutState {
  fanOut: FanOut | null;
  setFanOut: (f: FanOut | null) => void;
  clearFanOut: () => void;
}

/** Kept as its own tiny store (not folded into sessionStore) so the fan-out
 *  surface is fully self-contained and cheap to subscribe to. Fed by
 *  `syncFanOut` from the session snapshot, which is the projection of
 *  per-child `subagent_event` telemetry (see agent-core `Snapshot::fan_out`). */
export const useFanOutStore = create<FanOutState>((set) => ({
  fanOut: null,
  setFanOut: (f) => set({ fanOut: f }),
  clearFanOut: () => set({ fanOut: null }),
}));

/** A cheap content signature so we only push (and re-render the panel) when the
 *  fan-out actually changed, not on every re-cloned snapshot during streaming. */
function signature(f: FanOut | null | undefined): string {
  if (!f) return "";
  return `${f.total}:${f.done}:${f.running}:${f.agents.map((a) => a.id + a.status).join(",")}`;
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
  const agents: FanOutAgent[] = [
    { id: "environment", label: "Prepare the test environment", status: "done" },
    { id: "implementation", label: "Implement the repository changes", status: "running" },
    { id: "verification", label: "Review and run the full test suite", status: "queued" },
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
