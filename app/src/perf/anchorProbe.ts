/** Per-frame geometry sampling for a small set of anchor elements.
 *
 *  Stands in for the Layout Instability API, which the platform WebView does
 *  not implement. Rather than a single opaque CLS score, this reports what the
 *  eye actually notices during a transition: how far an element travelled, and
 *  how many times it REVERSED direction.
 *
 *  The reversal count is the point. Smooth motion is monotonic; an element that
 *  moves down, then up, then down again inside one transition is the "twinkle"
 *  or "jitter" a user reports, and counting reversals turns that complaint into
 *  a falsifiable number.
 *
 *  Generalized from `harness/motion-exit-probe.mjs`, which already samples
 *  opacity/transform per frame and runs against WebKit. */

export interface AnchorFrame {
  t: number;
  x: number;
  y: number;
  width: number;
  height: number;
  opacity: number;
  transform: string;
}

export interface AnchorTrack {
  selector: string;
  found: boolean;
  frames: AnchorFrame[];
}

export interface AnchorAnalysis {
  selector: string;
  frames: number;
  /** Sum of per-frame movement — total distance travelled, in CSS pixels. */
  totalDisplacementPx: number;
  maxFrameDisplacementPx: number;
  /** Times the vertical or horizontal direction of travel flipped. */
  directionReversals: number;
  /** Frames whose transform was not `none` — i.e. genuinely animating. */
  spatialFrames: number;
  maxFrameGapMs: number;
}

export interface TransitionProbe {
  name: string;
  durationMs: number;
  frameCount: number;
  maxFrameGapMs: number;
  anchors: AnchorAnalysis[];
}

function sampleAnchor(element: Element): AnchorFrame {
  const rect = element.getBoundingClientRect();
  const style = window.getComputedStyle(element);
  return {
    t: performance.now(),
    x: rect.x,
    y: rect.y,
    width: rect.width,
    height: rect.height,
    opacity: Number.parseFloat(style.opacity),
    transform: style.transform,
  };
}

function sign(value: number, epsilon = 0.25): -1 | 0 | 1 {
  if (value > epsilon) return 1;
  if (value < -epsilon) return -1;
  return 0;
}

export function analyzeAnchor(track: AnchorTrack): AnchorAnalysis {
  const { frames } = track;
  let total = 0;
  let maxStep = 0;
  let reversals = 0;
  let spatial = 0;
  let maxGap = 0;
  let previousX: -1 | 0 | 1 = 0;
  let previousY: -1 | 0 | 1 = 0;
  for (let i = 1; i < frames.length; i += 1) {
    const previous = frames[i - 1];
    const current = frames[i];
    const dx = current.x - previous.x;
    const dy = current.y - previous.y;
    const step = Math.hypot(dx, dy);
    total += step;
    maxStep = Math.max(maxStep, step);
    maxGap = Math.max(maxGap, current.t - previous.t);
    if (current.transform && current.transform !== "none") spatial += 1;
    const sx = sign(dx);
    const sy = sign(dy);
    // Only a genuine flip counts; settling to rest is not a reversal.
    if ((sx !== 0 && previousX !== 0 && sx !== previousX)
      || (sy !== 0 && previousY !== 0 && sy !== previousY)) {
      reversals += 1;
    }
    if (sx !== 0) previousX = sx;
    if (sy !== 0) previousY = sy;
  }
  return {
    selector: track.selector,
    frames: frames.length,
    totalDisplacementPx: total,
    maxFrameDisplacementPx: maxStep,
    directionReversals: reversals,
    spatialFrames: spatial,
    maxFrameGapMs: maxGap,
  };
}

/** Run `trigger`, then sample the anchors every frame until motion settles. */
export async function probeTransition(options: {
  name: string;
  trigger: () => void | Promise<void>;
  anchors: string[];
  /** Stop after this many frames even if something animates forever. */
  maxFrames?: number;
  /** Stop once no anchor has moved for this many consecutive frames. */
  settleFrames?: number;
}): Promise<TransitionProbe> {
  const { name, trigger, anchors, maxFrames = 400, settleFrames = 12 } = options;
  const tracks: AnchorTrack[] = anchors.map((selector) => ({
    selector,
    found: false,
    frames: [],
  }));

  const started = performance.now();
  let frameCount = 0;
  let maxFrameGapMs = 0;
  let previousFrameAt = started;
  let still = 0;
  // The settle exit is only armed once the trigger has resolved. Without this,
  // a trigger that spends a few frames awaiting (a store action, a network
  // round trip) lets the sampler watch `settleFrames` motionless frames and
  // resolve before the transition has even started — reporting a clean pass
  // over nothing.
  let triggered = false;

  const done = new Promise<void>((resolve) => {
    const step = () => {
      const now = performance.now();
      maxFrameGapMs = Math.max(maxFrameGapMs, now - previousFrameAt);
      previousFrameAt = now;
      frameCount += 1;
      let moved = false;
      for (const track of tracks) {
        const element = document.querySelector(track.selector);
        if (!element) continue;
        track.found = true;
        const frame = sampleAnchor(element);
        const last = track.frames[track.frames.length - 1];
        if (last && (Math.abs(frame.x - last.x) > 0.25 || Math.abs(frame.y - last.y) > 0.25
          || Math.abs(frame.opacity - last.opacity) > 0.01 || frame.transform !== last.transform)) {
          moved = true;
        }
        track.frames.push(frame);
      }
      still = moved ? 0 : still + 1;
      if (frameCount >= maxFrames || (triggered && still >= settleFrames)) {
        resolve();
        return;
      }
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  });

  await trigger();
  // Restart the stillness count from the moment the trigger settled, so frames
  // that elapsed while it was awaiting cannot count toward the exit.
  still = 0;
  triggered = true;
  await done;

  return {
    name,
    durationMs: performance.now() - started,
    frameCount,
    maxFrameGapMs,
    anchors: tracks.map(analyzeAnchor),
  };
}
