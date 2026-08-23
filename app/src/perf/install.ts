/** Wires the recorder onto `window.__clarkPerf`.
 *
 *  Present only when built with `VITE_PERF_HOOKS=1`; every normal build aliases
 *  `@clark-perf` to `noop.ts` instead, so none of this reaches a shipped
 *  bundle. `harness/product-boundary.spec.mjs` asserts that.
 *
 *  The recorder attaches from the OUTSIDE — a second `snapshot` listener and a
 *  store subscription — rather than adding marks inside the store or the
 *  bridge. Two reasons: the measured code path stays byte-identical to the
 *  shipped one, and the store's action modules do not grow. */

import { BlockProbe } from "./blockProbe";
import { FrameSampler, measureBaselinePeriod, type FrameRun } from "./frameSampler";
import { SnapshotPathRecorder } from "./snapshotPath";
import { buildSummary, type Budgets, type Summary } from "./report";
import { probeCapabilities, type Capabilities } from "./capabilities";
import { probeTransition, type TransitionProbe } from "./anchorProbe";
import { replayStream, type ReplayOptions } from "./streamReplay";

interface StoreLike {
  subscribe: (listener: (state: unknown) => void) => () => void;
  getState: () => { snapshot?: { timeline?: unknown[]; tool_calls?: object } };
  setState: (partial: Record<string, unknown>) => void;
}

interface InstallOptions {
  store: StoreLike;
  getBridge?: unknown;
}

interface ActiveRun {
  scenario: string;
  frames: FrameSampler;
  blocks: BlockProbe;
  path: SnapshotPathRecorder;
  mutations: number;
  observer: MutationObserver | null;
}

function isUnderHost(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Persist a report. Prefers the host command; always logs a copy so a run's
 *  stdout contains the data even when no directory is configured. */
async function persist(name: string, payload: unknown, echo = false): Promise<void> {
  // The host's report validator accepts only lowercase [a-z0-9._-]; scenario
  // names are caller-supplied ("streamA1"), so sanitize here instead of letting
  // the write fail silently under the real host.
  name = name.toLowerCase().replace(/[^a-z0-9._-]/g, "-").replace(/^[.-]+/, "");
  const json = JSON.stringify(payload, null, 2);
  // Echo only the summary. The per-sample streams run to tens of thousands of
  // entries, and serializing them to the console costs more than the frames it
  // is reporting on.
  if (echo) console.log(`CLARK_PERF_JSON ${name} ${JSON.stringify(payload)}`);
  if (!isUnderHost()) return;
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("perf_write_report", { name, json });
  } catch (error) {
    console.warn("perf report not written to disk", error);
  }
}

export function installPerfHooks(options: InstallOptions): void {
  const { store } = options;
  const capabilities: Capabilities = probeCapabilities();
  let active: ActiveRun | null = null;

  const api = {
    capabilities,

    /** The cadence this window actually got, not the one the config claims.
     *  A run whose baseline disagrees with the expected period is invalid. */
    measureBaselinePeriod,

    async start(scenario = "unnamed"): Promise<void> {
      if (active) throw new Error(`a run is already active: ${active.scenario}`);
      const frames = new FrameSampler();
      const blocks = new BlockProbe();
      const path = new SnapshotPathRecorder(store);
      const run: ActiveRun = { scenario, frames, blocks, path, mutations: 0, observer: null };
      // Idle scenarios assert zero mutations; counting them costs nothing and
      // turns "the app is busy when it should be still" into a number.
      const root = document.getElementById("root");
      if (root && typeof MutationObserver !== "undefined") {
        run.observer = new MutationObserver((records) => {
          run.mutations += records.length;
        });
        run.observer.observe(root, { subtree: true, childList: true, characterData: true });
      }
      await path.start();
      blocks.start();
      frames.start();
      active = run;
    },

    async stop(budgets?: Partial<Budgets>): Promise<Summary & { rootMutations: number }> {
      if (!active) throw new Error("no active run");
      const run = active;
      active = null;
      const frameRun: FrameRun = run.frames.stop();
      const blockSamples = run.blocks.stop();
      const { samples, clock } = run.path.stop();
      run.observer?.disconnect();
      const summary = {
        ...buildSummary({
          scenario: run.scenario,
          capabilities,
          frames: frameRun,
          blocks: blockSamples,
          snapshotPath: samples,
          clock,
          budgets,
        }),
        rootMutations: run.mutations,
      };
      await persist(`summary-${run.scenario}`, summary, true);
      await persist(`frames-${run.scenario}`, frameRun.samples);
      await persist(`blocks-${run.scenario}`, blockSamples);
      await persist(`snapshot-path-${run.scenario}`, samples);
      return summary;
    },

    /** Scenario A1: a deterministic streaming run through the real UI. */
    replayStream(replay?: ReplayOptions) {
      return replayStream(store, replay);
    },

    /** Scenario C: one transition, sampled every frame. */
    probeTransition,

    async probeTransitions(
      transitions: Array<{ name: string; trigger: () => void | Promise<void>; anchors: string[] }>,
    ): Promise<TransitionProbe[]> {
      const results: TransitionProbe[] = [];
      for (const transition of transitions) {
        results.push(await probeTransition(transition));
      }
      await persist("transitions", results);
      return results;
    },

    /** Scenario B: prove the app is doing nothing when nothing is happening. */
    async idleWindow(durationMs = 30_000) {
      await api.start("idle");
      await new Promise((resolve) => setTimeout(resolve, durationMs));
      return api.stop({ droppedRatio: 0, blockP99Ms: 16, blockMaxMs: 16 });
    },
  };

  (window as unknown as Record<string, unknown>).__clarkPerf = api;
  console.log("[clark-perf] recorder installed", {
    missingEntryTypes: capabilities.missingEntryTypes,
    clockResolutionMs: capabilities.clockResolutionMs,
  });
}
