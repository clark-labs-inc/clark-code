//! The app's motion vocabulary — one easing curve and a small speed scale, so
//! every animation reads as one system instead of a pile of inline magic
//! numbers. Mirrors the CSS tokens in `index.css` (`--dur-*`, `--ease-clark*`);
//! keep the two in sync. motion/react wants seconds and raw cubic-bezier
//! arrays, which is what these expose.

import type { Transition } from "motion/react";

/** Durations in SECONDS (motion/react's unit). Mirrors CSS `--dur-*` (ms). */
export const DUR = {
  /** Hovers, taps, small icon/color swaps. */
  fast: 0.12,
  /** The default — enters, popovers, most UI motion. */
  base: 0.2,
  /** Large surfaces, deliberate reveals. */
  slow: 0.3,
} as const;

/** Easing curves. `out` is the house ease-out (fast start, soft landing);
 *  `inOut` is for a value moving symmetrically between two states. */
export const EASE = {
  out: [0.22, 1, 0.36, 1] as const,
  inOut: [0.4, 0, 0.2, 1] as const,
};

/** The default transition every enter/appear should use unless it has a
 *  reason not to. */
export const TRANSITION: Transition = { duration: DUR.base, ease: EASE.out };

/** Fade only — no movement. For overlays, popovers, and anything where a
 *  positional shift would fight the layout. */
export const FADE = {
  initial: { opacity: 0 },
  animate: { opacity: 1 },
  exit: { opacity: 0 },
  transition: { duration: DUR.fast, ease: EASE.out },
} as const;

/** A gentle rise: fade + a few px up. The canonical "a card/row appeared"
 *  motion. Uses transform (`y`), never layout height, so it composites on the
 *  GPU and never triggers a reflow. */
export const RISE = {
  initial: { opacity: 0, y: 6 },
  animate: { opacity: 1, y: 0 },
  exit: { opacity: 0, y: 4, transition: { duration: DUR.fast, ease: EASE.inOut } },
  transition: TRANSITION,
} as const;

/** Reduced-motion counterpart to any of the above: truly instant. No opacity
 *  keyframe — a 0-duration fade still paints a partial-opacity frame in
 *  WKWebView, which reads as flicker. Motion snaps straight to the end state. */
export const INSTANT = {
  initial: false,
  animate: { opacity: 1, y: 0, height: "auto" as const },
  exit: { opacity: 0, transition: { duration: 0 } },
  transition: { duration: 0 },
} as const;
