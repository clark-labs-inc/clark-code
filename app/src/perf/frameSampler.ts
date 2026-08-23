/** Records the interval between animation frames.
 *
 *  Frame *interval* is the honest proxy for perceived smoothness available
 *  inside a WebView: there is no presentation-time API here, so we cannot know
 *  when a frame reached the glass, only when the engine handed us the callback.
 *  A gap of two or more display periods means at least one frame's worth of
 *  work did not make it out on time.
 *
 *  The baseline period is MEASURED, never assumed. This machine has a 120 Hz
 *  variable-refresh panel and a 60 Hz external one, and a config file cannot
 *  tell you which cadence a given window actually got. */

export interface FrameSample {
  /** `performance.now()` at the callback. */
  t: number;
  /** Milliseconds since the previous callback. */
  dt: number;
}

export interface FrameRun {
  samples: FrameSample[];
  /** Median interval over the run — the cadence actually observed. */
  baselinePeriodMs: number;
  startedAt: number;
  stoppedAt: number;
}

/** Measure the idle frame cadence before trusting any budget derived from it. */
export function measureBaselinePeriod(durationMs = 2000): Promise<number> {
  return new Promise((resolve) => {
    const intervals: number[] = [];
    let previous = performance.now();
    const deadline = previous + durationMs;
    const step = () => {
      const now = performance.now();
      intervals.push(now - previous);
      previous = now;
      if (now < deadline) {
        requestAnimationFrame(step);
        return;
      }
      // Drop the first few intervals: the first callback after scheduling is
      // not representative of steady-state cadence.
      const steady = intervals.slice(3).sort((a, b) => a - b);
      resolve(steady.length === 0 ? 16.67 : steady[Math.floor(steady.length / 2)]);
    };
    requestAnimationFrame(step);
  });
}

export class FrameSampler {
  private samples: FrameSample[] = [];
  private handle: number | null = null;
  private previous = 0;
  private startedAt = 0;
  private stoppedAt = 0;

  /** @param maxSamples bounds memory on a long run; 60 Hz for 10 minutes. */
  constructor(private readonly maxSamples = 36_000) {}

  start(): void {
    if (this.handle !== null) return;
    this.samples = [];
    this.previous = performance.now();
    this.startedAt = this.previous;
    const step = () => {
      const now = performance.now();
      if (this.samples.length < this.maxSamples) {
        this.samples.push({ t: now, dt: now - this.previous });
      }
      this.previous = now;
      this.handle = requestAnimationFrame(step);
    };
    this.handle = requestAnimationFrame(step);
  }

  stop(): FrameRun {
    if (this.handle !== null) {
      cancelAnimationFrame(this.handle);
      this.handle = null;
    }
    this.stoppedAt = performance.now();
    const steady = this.samples.slice(1).map((s) => s.dt).sort((a, b) => a - b);
    return {
      samples: this.samples,
      baselinePeriodMs: steady.length === 0 ? 16.67 : steady[Math.floor(steady.length / 2)],
      startedAt: this.startedAt,
      stoppedAt: this.stoppedAt,
    };
  }

  get running(): boolean {
    return this.handle !== null;
  }
}
