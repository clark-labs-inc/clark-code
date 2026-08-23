/** What this engine can actually measure.
 *
 *  The recorder's target is the platform WebView, where several performance
 *  APIs the web has standardized around simply do not exist: Long Tasks,
 *  Layout Instability, Largest Contentful Paint, and Event Timing are all
 *  Chromium-only. Silently observing nothing would produce a report full of
 *  confident zeroes, so every capability is probed and written into the run
 *  manifest — a later reader can then tell a real measurement from a
 *  substituted one. */

/** Entry types the recorder asks for, and what fills in when they are absent. */
export const DESIRED_ENTRY_TYPES = [
  "mark",
  "measure",
  "navigation",
  "paint",
  "resource",
  "longtask",
  "layout-shift",
  "largest-contentful-paint",
  "event",
] as const;

export interface Capabilities {
  /** Entry types this engine admits, from `supportedEntryTypes`. */
  supportedEntryTypes: string[];
  /** Requested types this engine does NOT support — the substitution list. */
  missingEntryTypes: string[];
  /** True when the recorder must fall back to its own occupancy probe. */
  needsBlockProbeForLongTasks: boolean;
  /** True when the recorder must fall back to anchor displacement sampling. */
  needsAnchorProbeForLayoutShift: boolean;
  /** Distinct values seen across many back-to-back `performance.now()` reads.
   *  WebKit deliberately coarsens the clock; this is the floor under every
   *  duration in the report, and quoting a number finer than it is a lie. */
  clockResolutionMs: number;
  hardwareConcurrency: number | null;
  devicePixelRatio: number;
  screen: { width: number; height: number; availWidth: number; availHeight: number } | null;
  prefersReducedMotion: boolean;
  userAgent: string;
}

/** Measure the effective `performance.now()` granularity.
 *
 *  Spins until the clock actually advances rather than sampling a fixed count:
 *  WebKit coarsens the clock enough that a thousand back-to-back reads can all
 *  return the same value, which would report infinite resolution instead of the
 *  real (coarse) one. */
function clockResolutionMs(ticksWanted = 20, budgetMs = 50): number {
  const gaps: number[] = [];
  const deadline = performance.now() + budgetMs;
  let previous = performance.now();
  while (gaps.length < ticksWanted && performance.now() < deadline) {
    const value = performance.now();
    if (value !== previous) {
      gaps.push(value - previous);
      previous = value;
    }
  }
  if (gaps.length === 0) return Number.POSITIVE_INFINITY;
  return Math.min(...gaps);
}

export function probeCapabilities(): Capabilities {
  const supported: string[] = [
    ...(((PerformanceObserver as unknown as { supportedEntryTypes?: string[] })
      .supportedEntryTypes) ?? []),
  ];
  const missing = DESIRED_ENTRY_TYPES.filter((type) => !supported.includes(type));
  return {
    supportedEntryTypes: supported,
    missingEntryTypes: missing,
    needsBlockProbeForLongTasks: !supported.includes("longtask"),
    needsAnchorProbeForLayoutShift: !supported.includes("layout-shift"),
    clockResolutionMs: clockResolutionMs(),
    hardwareConcurrency: navigator.hardwareConcurrency ?? null,
    devicePixelRatio: window.devicePixelRatio,
    screen: window.screen
      ? {
        width: window.screen.width,
        height: window.screen.height,
        availWidth: window.screen.availWidth,
        availHeight: window.screen.availHeight,
      }
      : null,
    prefersReducedMotion: window.matchMedia("(prefers-reduced-motion: reduce)").matches,
    userAgent: navigator.userAgent,
  };
}

/** Subscribe to an entry type without assuming the engine admits it.
 *  Returns a disconnect function, or null when the type is unsupported —
 *  callers must treat null as "substitute a fallback", never as "zero events". */
export function observeIfSupported(
  type: string,
  handler: (entries: PerformanceEntry[]) => void,
): (() => void) | null {
  try {
    const observer = new PerformanceObserver((list) => handler(list.getEntries()));
    // `buffered` is itself unevenly supported; a throw here means unsupported.
    observer.observe({ type, buffered: true } as PerformanceObserverInit);
    return () => observer.disconnect();
  } catch {
    return null;
  }
}
