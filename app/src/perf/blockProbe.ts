/** Measures how long the main thread was unavailable.
 *
 *  This stands in for the Long Tasks API, which the platform WebView does not
 *  implement. A `MessageChannel` that re-posts to itself from its own handler
 *  forms a macrotask loop that runs as soon as the main thread is free, so the
 *  gap between consecutive ticks *is* the time something else held the thread.
 *
 *  `MessageChannel` rather than `setTimeout(…, 0)` because WebKit clamps timers
 *  to a 4 ms floor and throttles them further under load — which would put the
 *  measurement floor above the frame budget we care about. Port messages carry
 *  no such clamp.
 *
 *  Read this together with `FrameSampler`. The cross-reference is what makes
 *  the result diagnostic rather than merely descriptive:
 *    - a frame gap WITH a matching block  -> our JavaScript held the thread
 *    - a frame gap WITHOUT a block        -> the frame was lost downstream
 *      (compositor, window server, display handoff) and is not ours to fix. */

export interface BlockSample {
  /** `performance.now()` at the end of the gap. */
  t: number;
  /** Length of the gap in milliseconds. */
  ms: number;
}

export class BlockProbe {
  private channel: MessageChannel | null = null;
  private samples: BlockSample[] = [];
  private previous = 0;
  private stopped = true;

  /** @param thresholdMs gaps below this are normal scheduling, not blocking.
   *  @param maxSamples bounds memory on a long run. */
  constructor(
    private readonly thresholdMs = 8,
    private readonly maxSamples = 20_000,
  ) {}

  start(): void {
    if (!this.stopped) return;
    this.stopped = false;
    this.samples = [];
    this.previous = performance.now();
    const channel = new MessageChannel();
    this.channel = channel;
    channel.port1.onmessage = () => {
      if (this.stopped) return;
      const now = performance.now();
      const gap = now - this.previous;
      this.previous = now;
      if (gap >= this.thresholdMs && this.samples.length < this.maxSamples) {
        this.samples.push({ t: now, ms: gap });
      }
      channel.port2.postMessage(0);
    };
    channel.port2.postMessage(0);
  }

  stop(): BlockSample[] {
    this.stopped = true;
    if (this.channel) {
      this.channel.port1.onmessage = null;
      this.channel.port1.close();
      this.channel.port2.close();
      this.channel = null;
    }
    return this.samples;
  }

  /** Read the samples so far without ending the run. */
  peek(): BlockSample[] {
    return [...this.samples];
  }
}
