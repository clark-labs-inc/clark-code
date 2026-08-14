# Motion design system

Clark Code uses motion to explain state and preserve spatial context. It must
never delay work, compete with reading, or become the only way status is
communicated.

## Sources of truth

- `src/lib/motion.ts` owns React motion presets, easing, duration, stagger,
  indeterminate progress, chat entrances, and reduced-motion counterparts.
- `src/index.css` owns the matching CSS tokens and WebView-specific effects.
- `src/main.tsx` applies `MotionConfig reducedMotion="user"` to the whole app.
- `src/surfaces/Toast.tsx` wraps Sonner's accessible notification queue in
  Clark tokens; `src/index.css` owns its visible style and cadence.
- `src/lib/motionPolicy.spec.ts` prevents local motion dialects from drifting
  back into feature code.

## Vocabulary

| Token | Time | Use |
| --- | ---: | --- |
| `fast` | 120 ms | Hover, press, icon and color feedback, compact exits |
| `base` | 200 ms | Menus, dialogs, rows, most entrances and state changes |
| `slow` | 300 ms | Large surfaces and deliberate reveals |

CSS uses `duration-fast`, `duration-base`, and `duration-slow`. The unqualified
Tailwind `transition` utility defaults to `base` and the house ease-out. React
uses `DUR`, `EASE`, and the exported presets from `src/lib/motion.ts`.

The default ease is `cubic-bezier(0.22, 1, 0.36, 1)`: respond immediately and
land softly. Symmetric movement uses `EASE.inOut`.

## Rules

1. Animate opacity and compositor transforms for entrances and exits. Do not
   animate page-size overlays spatially or use layout properties for decoration.
2. Prefer `FADE`, `RISE`, `RISE_SMALL`, `DIALOG`, `SLIDE_LEFT`, `SLIDE_RIGHT`,
   and `EXPAND` over inline values.
3. Use `layout` only when movement preserves meaningful spatial context. Disable
   it when reduced motion is active.
4. Use `staggeredTransition` for finite choreography and
   `indeterminateTransition` only for a mounted, genuinely pending state.
5. Do not use `transition-all`, numeric duration utilities, hand-rolled stagger
   arithmetic, or feature-local infinite Motion loops.
6. Replayed history mounts settled. Only newly appended transcript rows enter.
   Streaming words animate in normal motion and render immediately under reduced
   motion.
7. Motion is enhancement, never status. Busy, success, failure, selection, and
   progress states need text, semantics, shape, or color that remains useful when
   animations are disabled.
8. Toasts go through the single Sonner host. Reuse stable IDs for repeatable
   feedback, keep notices and warnings in the existing store channels, and do
   not mount feature-local toaster stacks.

## Reduced motion

`prefers-reduced-motion: reduce` is an application-wide contract:

- Motion for React disables transform and layout motion.
- Feature presets use `accessibleMotion`, `REDUCED_EXIT`, or their explicit
  reduced counterpart.
- New content may use a short opacity cue; spatial entrances and word-by-word
  streaming are removed.
- CSS animations run for at most one near-zero-duration iteration, known loading
  effects are disabled, and hover scale/translate affordances stay spatially
  still.
- A static visual and semantic status remains after continuous motion stops.

## WebView and performance

Tauri renders through WebView2 on Windows and WebKit on macOS and Linux. Keep
motion inside the common compositor-safe subset and verify real packaged builds
when introducing a new rendering technique.

- Use transform strings where a stable cross-WebView layer matters.
- Avoid permanent `will-change`; reserve it for mounted loading effects.
- Do not animate large shadows, filters, or full-viewport transforms.
- Avoid remounting streamed content and do not animate historical replay.

## Verification

Run from `app/`:

```bash
pnpm typecheck
pnpm test
pnpm build
```

Run the full/reduced browser frame probe from the repository root:

```bash
node harness/motion-exit-probe.mjs both
```

For visual changes, inspect start, popover, autocomplete, active-work,
completed-work, settings, keyboard-focus, and reduced-motion states at desktop
and narrow viewports. Check browser console errors and the packaged Tauri shell
before making a cross-platform release claim.
