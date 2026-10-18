# Dialog flicker reproduction — 2026-10-18

Two independent rendering boundaries failed in the actual React components.

1. **Presence ownership:** the workspace conditionally unmounted lazy modal
   shells along with their inner `AnimatePresence`. The palette returned early
   while closed, destroying its presence owner too. All five dialogs disappeared
   in 3–14 ms with zero intermediate exit frames in the initial Chromium probe.
   Closing and reopening 50 ms later created a different opacity-zero node.
2. **WebKit opacity handoff:** after repairing presence, every dialog still
   flickered on first appearance with reduced motion enabled. The universal CSS
   rule gave every element a nonzero opacity transition. When Motion committed
   its completed WAAPI fade's final inline opacity, WebKit started a CSS fade
   from zero. The retained palette samples go from opacity `0.999995` with
   inline opacity `0` and a WAAPI `Animation` at 123 ms, to opacity `0` with
   inline opacity `1` and an opacity `CSSTransition` at 143 ms. The same live
   node stayed in place. This is a measured style handoff, not inferred shadow
   or GPU rasterization behavior.

The workspace now retains keyed lazy shells until propagated exits finish.
The palette's presence owner stays mounted while its child leaves. Reduced
motion resets implicit CSS transition durations and grants brief durations to
explicit CSS transition utilities; color-only utilities retain their property
set. Motion owns its existing opacity fades without a competing CSS fade.

## Evidence

- [Initial presence failures](before-lifecycle.json) and
  [Chromium recording](before-lifecycle.webm): both motion preferences, all five
  dialogs; no entry-opacity probe existed in this first harness iteration.
- [WebKit appearance failures after the lifecycle repair](before-css.json) and
  [reduced-motion recording](before-css.webm): one entry-opacity reversal per
  dialog, including the animation-type handoff described above.
- [Final frame samples](after.json), [WebKit reduced-motion recording](after-webkit.webm),
  and [Chromium normal-motion recording](after-chromium.webm): 20 combinations,
  covering first appearance, settled exit, and rapid close/reopen.
- [Validation and source digests](validation.json) pin the final source and
  retained artifacts. Replay with `node harness/element-presence.mjs` or
  `pnpm --dir harness test:element-presence`.

Final acceptance has zero entry-opacity reversals or competing CSS opacity
transitions, continuous exits, one retained dialog node during rapid reopening,
zero settled positional displacement, preserved composer text, and zero page
errors across Chromium/WebKit and normal/reduced motion.

The shared-CSS follow-up also passed the transcript geometry/identity, workspace
navigation, image/attachment motion, and permission/toast exit browser lanes
in both engines with both motion preferences. Frozen frontend install,
typecheck, 848 frontend tests (3 skipped), production build, Rust formatting,
strict foundation Clippy, 802 Nextest tests (8 skipped), and core WASM check pass.

These are deterministic browser fixtures using a synthetic local account and
the mock bridge. The phone-access banner belongs to that fixture account.
Frame timings establish lifecycle/opacity behavior, not a frame-rate or latency
benchmark. The installed Clark Code app was left running; no packaged/native
interaction, native OS-window creation, live account, hosted model, or arbitrary
popover shadow rasterization was qualified. Packaged release qualification
belongs to the downstream product, alongside the earlier transcript-stability work.
