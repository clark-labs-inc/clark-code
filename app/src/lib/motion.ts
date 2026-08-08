//! The app's motion vocabulary — one easing curve and a small speed scale, so
//! every animation reads as one system instead of a pile of inline magic
//! numbers. Mirrors the CSS tokens in `index.css` (`--dur-*`, `--ease-agent*`);
//! keep the two in sync. motion/react wants seconds and raw cubic-bezier
//! arrays, which is what these expose.

import type { MotionProps, Transition } from "motion/react";

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
 * motion. A direct transform string gives every supported WebView the best
 * chance to keep it on the compositor; it never changes layout height. */
export const RISE = {
  initial: { opacity: 0, transform: "translateY(6px)" },
  animate: { opacity: 1, transform: "translateY(0)" },
  exit: {
    opacity: 0,
    transform: "translateY(-4px)",
    transition: { duration: DUR.fast, ease: EASE.inOut },
  },
  transition: TRANSITION,
} satisfies MotionProps;

/** The tight version of RISE for dense rows (sidebar list rows, work lines):
 * fade + 4px up. The exit is deliberately fade-only — dense rows don't drift
 * spatially as they leave, which is also the reduced-motion policy. */
export const RISE_SMALL = {
  initial: { opacity: 0, transform: "translateY(4px)" },
  animate: { opacity: 1, transform: "translateY(0)" },
  exit: {
    opacity: 0,
    transition: { duration: DUR.fast, ease: EASE.inOut },
  },
  transition: TRANSITION,
} satisfies MotionProps;

/** Expand/collapse for transient rows that participate in document flow. */
export const EXPAND = {
  initial: { opacity: 0, y: 6, height: 0 },
  animate: { opacity: 1, y: 0, height: "auto" as const },
  exit: {
    opacity: 0,
    height: 0,
    transition: { duration: DUR.base, ease: EASE.inOut },
  },
  transition: TRANSITION,
  style: { overflow: "hidden" },
} satisfies MotionProps;

/** Reduced-motion counterpart to EXPAND. Keeps a short opacity-only fade on
 * exit — a hard cut reads as a glitch — but never spatial movement; the enter
 * still snaps in. */
export const EXPAND_REDUCED = {
  initial: false,
  animate: { opacity: 1, y: 0, height: "auto" as const },
  exit: { opacity: 0, transition: { duration: DUR.fast } },
  transition: { duration: 0 },
} as const;

/** Full-viewport scrims should only fade. Moving a page-sized layer costs more
 * GPU memory and can trigger vestibular discomfort without adding meaning. */
export const OVERLAY = {
  initial: { opacity: 0 },
  animate: { opacity: 1 },
  exit: { opacity: 0 },
  transition: { duration: DUR.fast, ease: EASE.out },
} satisfies MotionProps;

/** Dialogs use a single transform string so WebKit, Chromium, and WebView2 can
 * keep the transition on the compositor instead of animating CSS variables
 * for independent transform properties. */
export const DIALOG = {
  initial: { opacity: 0, transform: "translateY(6px) scale(0.99)" },
  animate: { opacity: 1, transform: "translateY(0) scale(1)" },
  exit: { opacity: 0, transform: "translateY(3px) scale(0.995)" },
  transition: TRANSITION,
} satisfies MotionProps;

/** Small directional slides for tab/stage panel swaps. Content that enters the
 * stage from the left uses SLIDE_LEFT (-5px); content entering from the right
 * uses SLIDE_RIGHT (+5px). A single transform string keeps them on the
 * compositor, and accessibleMotion strips the spatial movement under reduce. */
export const SLIDE_LEFT = {
  initial: { opacity: 0, transform: "translateX(-5px)" },
  animate: { opacity: 1, transform: "translateX(0)" },
  exit: {
    opacity: 0,
    transform: "translateX(-4px)",
    transition: { duration: DUR.fast, ease: EASE.inOut },
  },
  transition: TRANSITION,
} satisfies MotionProps;

export const SLIDE_RIGHT = {
  initial: { opacity: 0, transform: "translateX(5px)" },
  animate: { opacity: 1, transform: "translateX(0)" },
  exit: {
    opacity: 0,
    transform: "translateX(4px)",
    transition: { duration: DUR.fast, ease: EASE.inOut },
  },
  transition: TRANSITION,
} satisfies MotionProps;

/** Apply one vocabulary everywhere while keeping the stronger the agent policy:
 * reduced motion removes spatial movement but keeps a short opacity-only fade
 * on exit so disappearing surfaces don't cut out like a glitch. */
export function accessibleMotion(
  preset: MotionProps,
  reduce: boolean | null,
): MotionProps {
  if (!reduce) return preset;
  return {
    initial: false,
    animate: { opacity: 1, transform: "none" },
    exit: { opacity: 0, transition: { duration: DUR.fast } },
    transition: { duration: 0 },
  };
}

/** Reduced-motion exit policy for surfaces that spread their own motion props:
 * a short opacity-only fade, never spatial movement. Unmounted surfaces that
 * hard-cut at duration 0 read as a glitch, so everything leaving the screen
 * fades out consistently while reveals still snap in. */
export const REDUCED_EXIT = {
  opacity: 0,
  transition: { duration: DUR.fast },
} as const;

/** The transition a surface should use for the `index`th row of a staggered
 * reveal. Under reduced motion the choreography snaps to zero (duration 0);
 * otherwise rows land `perIndex * index` apart at the given duration and ease.
 * Pass `overrides` for a per-site duration or a fixed delay (e.g. `{ duration:
 * DUR.slow }` or `{ delay: 0.08 }`). */
export function staggeredTransition(
  reduce: boolean | null,
  index: number,
  perIndex = 0.04,
  overrides?: Partial<Transition>,
): Transition {
  if (reduce) return { duration: 0 };
  return {
    duration: DUR.base,
    ease: EASE.out,
    delay: perIndex * index,
    ...overrides,
  };
}

/** Transcript text arrives more slowly than general UI chrome because it is a
 * reading surface. Values are milliseconds because Streamdown's API uses ms. */
export const CHAT_TEXT_ANIMATION = {
  animation: "fadeIn",
  duration: DUR.base * 1000,
  easing: "cubic-bezier(0.22, 1, 0.36, 1)",
  sep: "word",
  stagger: 18,
} as const;

/** Reduced motion preserves a readable opacity cue without spatial movement. */
export const CHAT_REDUCED_TEXT_ANIMATION = {
  animation: "fadeIn",
  duration: DUR.base * 1000,
  easing: "ease-out",
  sep: "word",
  stagger: 6,
} as const;

const USER_MESSAGE_ROW: MotionProps = {
  initial: { opacity: 0, transform: "translate3d(8px, 2px, 0) scale(0.995)" },
  animate: { opacity: 1, transform: "translate3d(0, 0, 0) scale(1)" },
  transition: TRANSITION,
};

const ASSISTANT_MESSAGE_ROW: MotionProps = {
  initial: { opacity: 0, transform: "translate3d(0, 7px, 0)" },
  animate: { opacity: 1, transform: "translate3d(0, 0, 0)" },
  transition: { duration: DUR.slow, ease: EASE.out },
};

const SYSTEM_MESSAGE_ROW: MotionProps = {
  initial: FADE.initial,
  animate: FADE.animate,
  transition: TRANSITION,
};

export const CHAT_REDUCED_ROW_MOTION: MotionProps = {
  initial: { opacity: 0 },
  animate: { opacity: 1 },
  transition: TRANSITION,
};

export type ChatMessageRole = "user" | "agent" | "system";

export function chatRowMotion(role: ChatMessageRole): MotionProps {
  if (role === "user") return USER_MESSAGE_ROW;
  if (role === "agent") return ASSISTANT_MESSAGE_ROW;
  return SYSTEM_MESSAGE_ROW;
}

export interface ChatRowMotionState {
  sessionId: string | null;
  committedKeys: Set<string>;
}

export function createChatRowMotionState(): ChatRowMotionState {
  return { sessionId: null, committedKeys: new Set() };
}

/** Only rows appended after the current conversation is committed may enter.
 * Replayed history and streaming updates to an existing key stay settled. */
export function enteringChatRowKeys(
  state: ChatRowMotionState,
  sessionId: string | undefined,
  rowKeys: readonly string[],
): ReadonlySet<string> {
  if (!sessionId || state.sessionId !== sessionId) return new Set();
  return new Set(rowKeys.filter((key) => !state.committedKeys.has(key)));
}

export function commitChatRowKeys(
  state: ChatRowMotionState,
  sessionId: string | undefined,
  rowKeys: readonly string[],
): void {
  if (!sessionId) {
    state.sessionId = null;
    state.committedKeys.clear();
    return;
  }
  if (state.sessionId !== sessionId) {
    state.sessionId = sessionId;
    state.committedKeys = new Set(rowKeys);
    return;
  }
  rowKeys.forEach((key) => state.committedKeys.add(key));
}
