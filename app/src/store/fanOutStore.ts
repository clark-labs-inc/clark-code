import { create } from "zustand";

/** One agent in a parallel fan-out run. */
export interface FanOutAgent {
  id: string;
  label: string;
  status: "queued" | "running" | "done" | "failed";
}

/** A live parallel fan-out: one job split across many cloud agents. `agents` is a
 *  render sample of the swarm (the first N tiles), not necessarily all `total`. */
export interface FanOut {
  title: string;
  total: number;
  done: number;
  running: number;
  agents: FanOutAgent[];
}

interface FanOutState {
  fanOut: FanOut | null;
  setFanOut: (f: FanOut | null) => void;
  clearFanOut: () => void;
}

/** Kept as its own tiny store (not folded into sessionStore) so the fan-out
 *  surface is fully self-contained and cheap to subscribe to. */
export const useFanOutStore = create<FanOutState>((set) => ({
  fanOut: null,
  setFanOut: (f) => set({ fanOut: f }),
  clearFanOut: () => set({ fanOut: null }),
}));

// INTEGRATION TODO: the parallel-agent fan-out has no event source yet. When
// provider-clark (or the runtime) emits fan-out progress, call
// `useFanOutStore.getState().setFanOut(...)` on each update and `clearFanOut()`
// when it finishes. Until then this stays null in production and <FanOutPanel/>
// renders nothing. `previewFanOut()` (exposed on window in dev) shows the surface.
export function previewFanOut(): void {
  const files = [
    "Button", "Card", "Modal", "Sidebar", "Input", "Table", "Toast", "Badge",
    "Tabs", "Menu", "Avatar", "Dialog", "Drawer", "Slider", "Chart",
  ];
  const agents: FanOutAgent[] = files.map((f, i) => ({
    id: String(i),
    label: `${f}.tsx`,
    status: i < 5 ? "done" : i < 13 ? "running" : "queued",
  }));
  useFanOutStore.getState().setFanOut({
    title: "Refactoring every component in src/ to the new design tokens",
    total: 240,
    done: 37,
    running: 203,
    agents,
  });
}

if (import.meta.env.DEV) {
  (window as unknown as { previewFanOut?: () => void }).previewFanOut = previewFanOut;
}
