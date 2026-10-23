# Rendering flicker replay — 2026-10-20

No current flicker was reproduced. The 25 rendering, harness, and dependency
source hashes from yesterday's passing replay still match. The current mobile
project changes and all other existing work were left unchanged. No app or
harness patch was applied.

The original first contract breaks remain documented in the
[historical reproduction](../2026-10-18/README.md): dialog presence owners
were destroyed before exits finished, and WebKit reduced motion started an
unintended CSS opacity fade after Motion's WAAPI fade completed. Re-reading the
retained palette trace confirms the same node changed from opacity 0.999995 at
123 ms to opacity 0 at 143 ms as ownership switched to a CSS transition.
Those are historical failing frames, not new failures in today's checkout.

## Current evidence

- Dialog frame samples (local `frames.json`): 2,041 animation-frame observations
  across 20 first-mount, settled-exit, and rapid-reopen cases. Five dialogs pass
  in Chromium/WebKit with normal/reduced motion. There are zero entry reversals,
  competing CSS opacity transitions, replaced nodes, settled displacement, or
  page errors. Every exit includes intermediate frames, with a minimum of three.
- Transcript results (local `transcript-results.json`): four engine/motion
  configurations, 12 commands each, and 59–72 sampled command frames per
  configuration. No overlap, replacement, duplicate rows, status/finish height
  shift, disrupted scroll ownership, or lost Markdown identity was found.
- [Validation](validation.json): source hashes, pre-existing change inventory,
  counts, scope, and check results. Frozen frontend installation, typecheck,
  **856 tests passed with 3 skipped**, and production build all pass.

The unchanged harnesses start isolated Vite servers on allocated ports with a
synthetic local account and mock bridge. Reproduce with
`node harness/element-presence.mjs` and `node harness/rendering-stability.mjs`.
Today's raw videos remain disposable in `target/flicker-followup-2026-10-20/`;
the durable evidence is the retained frame samples and this conclusion. Earlier
[recordings and screenshots](../2026-10-19/README.md) remain available.

No production app or user tab was controlled or restarted. No hosted model
calls, signing, commit, or release occurred. Native OS-window creation,
packaged WebView behavior, untested popovers, and frame-rate performance remain
unverified. The separate Rust upgrade's Clippy migration remains untouched.

Raw frame/transcript traces are local generated files, ignored by Git. The
versioned validation receipt and conclusion remain; rerun the listed harnesses
to reproduce the raw observations on another checkout.
