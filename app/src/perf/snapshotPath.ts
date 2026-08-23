/** Times the crossing from host emit to painted frame.
 *
 *  Four points, all on the path a streamed token actually travels:
 *
 *    emit   the host serialized the snapshot and handed it to the WebView
 *           (reported by the `perf-emit-tick` companion event)
 *    arrive the payload finished parsing and our listener ran
 *    commit the store applied it and React committed
 *    paint  two animation frames later, so layout and paint have happened
 *
 *  Every duration is also recorded against `timelineLen`, because the question
 *  is not just "how slow" but "does it get slower as the conversation grows".
 *  Work that scales with transcript length is the difference between a constant
 *  cost and one that degrades over a session, and only the regression shows it. */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

/** The host's companion tick. Mirrors `EmitTick` in `src-tauri/src/perf.rs`. */
interface EmitTick {
  seq: number;
  emit_unix_us: number;
  bytes: number;
  timeline_len: number;
  tool_calls_len: number;
}

export interface SnapshotPathSample {
  seq: number | null;
  /** Host-reported wire size, when a tick was paired with this arrival. */
  bytes: number | null;
  timelineLen: number;
  toolCallsLen: number;
  /** Host emit to listener entry, in ms. Null when no tick was paired. */
  emitToArriveMs: number | null;
  /** Listener entry to store commit, in ms. */
  arriveToCommitMs: number | null;
  /** Store commit to the second animation frame after it, in ms. */
  commitToPaintMs: number | null;
  arriveAt: number;
}

/** Bound on how far the host and WebView clocks disagree.
 *
 *  Without this, a three-millisecond `emitToArriveMs` could be entirely clock
 *  skew. Half the minimum round trip is the tightest honest bound available. */
export interface ClockOffset {
  offsetMs: number;
  errorBoundMs: number;
}

export async function calibrateClock(rounds = 5): Promise<ClockOffset> {
  let best = Number.POSITIVE_INFINITY;
  let offset = 0;
  for (let i = 0; i < rounds; i += 1) {
    const before = Date.now();
    const hostUs = await invoke<number>("perf_clock_probe");
    const after = Date.now();
    const rtt = after - before;
    if (rtt < best) {
      best = rtt;
      // Assume the host read its clock at the midpoint of our round trip.
      offset = hostUs / 1000 - (before + after) / 2;
    }
  }
  return { offsetMs: offset, errorBoundMs: best / 2 };
}

interface StoreLike {
  subscribe: (listener: (state: unknown) => void) => () => void;
  getState: () => { snapshot?: { timeline?: unknown[]; tool_calls?: object } };
}

type SnapshotRef = { timeline?: unknown[]; tool_calls?: object } | undefined;

export class SnapshotPathRecorder {
  private samples: SnapshotPathSample[] = [];
  private unlisteners: UnlistenFn[] = [];
  private unsubscribe: (() => void) | null = null;
  private pendingTicks: EmitTick[] = [];
  private lastArrival: { at: number; tick: EmitTick | null } | null = null;
  private lastSnapshot: SnapshotRef;
  private clock: ClockOffset = { offsetMs: 0, errorBoundMs: Number.POSITIVE_INFINITY };

  constructor(private readonly store: StoreLike, private readonly maxSamples = 20_000) {}

  async start(): Promise<void> {
    this.samples = [];
    try {
      this.clock = await calibrateClock();
    } catch {
      // No host (browser or mock run); emit-side timings stay null.
    }

    // The host sends the tick immediately before the snapshot, and all evals
    // share one queue, so the tick that arrives first belongs to the snapshot
    // that arrives next.
    try {
      this.unlisteners.push(
        await listen<EmitTick>("perf-emit-tick", (event) => {
          this.pendingTicks.push(event.payload);
          if (this.pendingTicks.length > 64) this.pendingTicks.shift();
        }),
      );
      this.unlisteners.push(
        await listen("snapshot", () => {
          this.lastArrival = { at: performance.now(), tick: this.pendingTicks.shift() ?? null };
        }),
      );
    } catch {
      // Not running under the host.
    }

    this.lastSnapshot = this.store.getState().snapshot;
    this.unsubscribe = this.store.subscribe(() => {
      // The store notifies on every state change — sidebar metadata, running
      // ids, selection — not just snapshot commits. Sampling those would both
      // pad the distributions with non-snapshot work and, worse, consume the
      // pending arrival tick that belongs to the NEXT real snapshot commit,
      // mis-pairing arrive -> commit. Only a snapshot identity change counts.
      const snapshot = this.store.getState().snapshot;
      if (snapshot === this.lastSnapshot) return;
      this.lastSnapshot = snapshot;
      const arrival = this.lastArrival;
      this.lastArrival = null;
      const commitAt = performance.now();
      const timelineLen = snapshot?.timeline?.length ?? 0;
      const toolCallsLen = snapshot?.tool_calls ? Object.keys(snapshot.tool_calls).length : 0;
      const tick = arrival?.tick ?? null;
      const sample: SnapshotPathSample = {
        seq: tick?.seq ?? null,
        bytes: tick?.bytes ?? null,
        timelineLen,
        toolCallsLen,
        emitToArriveMs: tick && arrival
          ? (arrival.at + performance.timeOrigin) - (tick.emit_unix_us / 1000 - this.clock.offsetMs)
          : null,
        arriveToCommitMs: arrival ? commitAt - arrival.at : null,
        commitToPaintMs: null,
        arriveAt: arrival?.at ?? commitAt,
      };
      if (this.samples.length < this.maxSamples) this.samples.push(sample);
      // Two frames: the first runs before paint, the second after it.
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          sample.commitToPaintMs = performance.now() - commitAt;
        });
      });
    });
  }

  stop(): { samples: SnapshotPathSample[]; clock: ClockOffset } {
    this.unsubscribe?.();
    this.unsubscribe = null;
    for (const unlisten of this.unlisteners) unlisten();
    this.unlisteners = [];
    return { samples: this.samples, clock: this.clock };
  }
}
