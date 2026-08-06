# GUI motion system

Clark Desktop uses [Motion for React](https://motion.dev/docs/react) for
stateful transitions and CSS for small, static effects. Motion is MIT-licensed,
works with React DOM in Tauri's macOS, Windows, and Linux WebViews, and gives us
one implementation for presence, layout, gestures, and reduced motion.

## Runtime contract

- `app/src/main.tsx` owns one strict `LazyMotion` boundary with `domMax`.
  `domMax` is required because Clark uses layout animations. Strict mode rejects
  accidentally importing the larger `motion` components.
- Components import declarative elements from `motion/react-m`:
  `import * as m from "motion/react-m"`. Hooks and `AnimatePresence` continue to
  come from `motion/react`.
- Shared duration, easing, and surface presets live in `app/src/lib/motion.ts`.
  CSS uses the matching `--dur-*` and `--ease-clark*` tokens in
  `app/src/index.css`.
- `app/src/lib/motionPolicy.spec.ts` prevents raw `motion` components,
  `transition-all`, inline exit hard-cuts, raw inline slides/stagger
  arithmetic, and JS↔CSS token drift from returning.

## Product rules

Motion should explain a state change, preserve spatial continuity, or confirm
an action. It should not decorate every update.

1. Prefer opacity and a direct `transform` string. These are the properties most
   likely to remain on the compositor across WKWebView, WebView2, and WebKitGTK.
2. Do not animate `width`, `height`, `top`, `left`, shadows, or filters on large
   surfaces. If continuity genuinely needs layout animation, scope `layout` to
   the smallest element and use `layout="position"` when size interpolation
   would distort content.
3. Use `AnimatePresence` only around stable, keyed children. Use `mode="wait"`
   for one-at-a-time screen or panel replacement; use `popLayout` only for lists
   where the surrounding content should reflow immediately.
4. Keep feedback quick: 120 ms for controls, 200 ms for normal entries, and
   300 ms only for a deliberate large reveal.
5. Do not add `will-change` by default. Persistent compositor layers consume
   memory; add it only after a measured paint/composite bottleneck and remove it
   when the animation ends.
6. Avoid infinite motion unless it communicates ongoing work. Loading and
   streaming states must remain understandable when animation is disabled.

## The one vocabulary

Every animated surface expresses motion through `app/src/lib/motion.ts` —
spread a preset (`{...FADE}`) or route it through the helpers. Never write a
fresh inline `initial`/`animate`/`exit`/`transition` object with your own magic
numbers; `motionPolicy.spec.ts` enforces this by source scan.

Spread the full preset, then use the helpers for the parts that vary:

- `accessibleMotion(preset, reduce)` — the single reduced-motion gate. When
  `reduce` is true it returns `{ initial: false, animate: { opacity: 1,
  transform: "none" }, exit: { opacity: 0, transition: { duration: fast } },
  transition: { duration: 0 } }`, i.e. the enter snaps in and the exit is a
  short opacity-only fade with no spatial movement. When `reduce` is false it
  returns the chosen preset unchanged. Dialogs, overlays, and reduced-motion
  choreography should route here.
- `staggeredTransition(reduce, index, perIndex = 0.04, overrides?)` — the
  transition for the `index`-th row of a stagger. Under full motion it returns
  `{ duration: base, ease: out, delay: perIndex * index }`; under reduced
  motion it returns `{ duration: 0 }` (no choreography). Pass `overrides` for a
  per-site duration or a fixed delay, e.g.
  `staggeredTransition(reduce, 0, 0.04, { duration: DUR.slow })` or
  `staggeredTransition(reduce, 0, 0.04, { delay: 0.08 })`.
- `REDUCED_EXIT` — `{ opacity: 0, transition: { duration: fast } }`, for the
  small number of bespoke surfaces that spread their own props and must keep the
  reduced-motion fade-out exit.

### Preset catalog

| Preset | Enter | Exit | Transition |
| --- | --- | --- | --- |
| `FADE` | `{ opacity: 0 }` | `{ opacity: 0 }` | fast · out |
| `RISE` | `{ opacity: 0, translateY(6px) }` | `{ opacity: 0, translateY(-4px) }` | base · out |
| `RISE_SMALL` | `{ opacity: 0, translateY(4px) }` | `{ opacity: 0 }` (fade-only) | base · out |
| `SLIDE_LEFT` | `{ opacity: 0, translateX(-5px) }` | `{ opacity: 0, translateX(-4px) }` | base · out |
| `SLIDE_RIGHT` | `{ opacity: 0, translateX(5px) }` | `{ opacity: 0, translateX(4px) }` | base · out |
| `EXPAND` | `{ opacity: 0, y: 6, height: 0 }` | `{ opacity: 0, height: 0 }` | base · inOut |
| `OVERLAY` | `{ opacity: 0 }` | `{ opacity: 0 }` | fast · out |
| `DIALOG` | `{ opacity: 0, translateY(6px) scale(0.99) }` | `{ opacity: 0, translateY(3px) scale(0.995) }` | base · out |

`EXPAND_REDUCED` is the reduced-motion counterpart to `EXPAND`: enter snaps,
exit is an opacity-only fade. Use it with the same ternary as the transients in
`Conversation.tsx`: `{...(reduce ? EXPAND_REDUCED : EXPAND)}`.

The speed scale mirrors CSS: `DUR.fast = 120ms`, `DUR.base = 200ms`,
`DUR.slow = 300ms`; `EASE.out = cubic-bezier(0.22, 1, 0.36, 1)` and
`EASE.inOut = cubic-bezier(0.4, 0, 0.2, 1)` match `--dur-*` / `--ease-clark*`
in `index.css`. `motionPolicy.spec.ts` reads `index.css` and asserts the two
stay in sync.

Deliberate normalizations on top of the prior ad-hoc values (all intentional,
matching the canonical presets):

- Sidebar list rows: exit was `{ opacity: 0, y: -2 }` → now `RISE_SMALL`'s
  fade-only exit (no spatial exit).
- Sidebar selection toolbar: enter was `y: 8` → now `RISE` (`y: 6`).
- Same-file reveals collapsed onto one preset: `y: 3/4` → `RISE_SMALL`;
  `y: 5/6/7/8/10/12` → `RISE` (or `RISE` + `staggeredTransition` for duration
  overrides); toast/chip scale entries → `FADE`.
- Specialist tab/stage panels: `x: ±5` → `SLIDE_LEFT`/`SLIDE_RIGHT` (the one
  `x: 6` outlier normalized to `+5`).

## Accessibility

The root `MotionConfig reducedMotion="user"` follows the operating-system
preference. Components that choreograph presence also call `useReducedMotion`
and send the result through `accessibleMotion`, so Clark's policy is:

- **Enters snap in.** Reduced motion keeps content appearing immediately — no
  slides, no rises, no choreographed stagger, no scale.
- **Exits are a short opacity-only fade.** A disappearing surface fades out over
  120 ms instead of hard-cutting at `duration: 0`; a glitch cut reads as a bug,
  and the fade requires no spatial movement.

Under reduce, `accessibleMotion` and `staggeredTransition` (and `REDUCED_EXIT`
for bespoke spreads) are what guarantee both halves of that policy. The global
`prefers-reduced-motion: reduce` block disables CSS animation, transitions, and
smooth scrolling while preserving semantic status text.

Do not use movement as the only signal. Focus, labels, live-region text, color,
and icons must continue to communicate the same state.

## Review checklist

- Does the motion clarify a user-visible state change?
- Are only opacity and transform animated?
- Does the surface express motion through `lib/motion.ts` (a preset spread or
  `accessibleMotion`/`staggeredTransition`/`REDUCED_EXIT`) rather than inline
  `initial`/`animate`/`exit` magic numbers?
- Is `accessibleMotion` (or `REDUCED_EXIT`) actually used, so the reduced-motion
  exit fades out rather than hard-cutting?
- Are presence children keyed and actually unmounted?
- Does rapid repeated input settle on the final state without overlap?
- Does reduced motion make the same workflow immediately understandable?
- Has the transition been checked in the macOS WebView and at least one of
  Windows WebView2 or Linux WebKitGTK before release?
- Did the production bundle stay within the prior size envelope?

## Primary references

- [Motion: reduce bundle size](https://motion.dev/docs/react-reduce-bundle-size)
- [Motion: LazyMotion](https://motion.dev/docs/react-lazy-motion)
- [Motion: animation performance](https://motion.dev/docs/performance)
- [Motion: layout animation](https://motion.dev/docs/react-layout-animations)
- [Motion: AnimatePresence](https://motion.dev/docs/react-animate-presence)
- [Motion: reduced motion](https://motion.dev/docs/react-use-reduced-motion)
- [web.dev: animations and performance](https://web.dev/articles/animations-and-performance)
- [MDN: prefers-reduced-motion](https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/At-rules/@media/prefers-reduced-motion)
