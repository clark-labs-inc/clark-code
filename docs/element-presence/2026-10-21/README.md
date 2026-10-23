# Rendering flicker replay — 2026-10-21

No current flicker or new rendering contract break was reproduced at foundation
`921f564a8167c5cc329b948fbb8337dd6f1b3395`. All 32 sources covered by
yesterday's rendering receipt still match. New commits since that replay affect
Rust provider/worker behavior; the existing mobile and dependency changes were
preserved. No application or harness source was changed for this follow-up.

## Fresh observations

- Dialog frame data (local `frames.json`) retains 2,044 animation-frame observations
  across 20 first-mount, exit, and 50 ms close/reopen cases. Palette, new session,
  Settings, MCP, and SSH dialogs pass in Chromium/WebKit with normal/reduced
  motion. There are zero entry-opacity reversals, competing CSS opacity
  transitions, rapid-reopen node replacements, settled displacement, or page
  errors. Every exit has intermediate frames; the minimum is three.
- Transcript observations (local `transcript-results.json`) cover four engine/motion
  configurations and 12 command rows each, sampled across 57–72 frames per
  configuration. Overlap, duplicates, replacements, and status/finish height
  shifts are zero. History, scroll ownership, disclosure, short-answer geometry,
  and Markdown identity checks also pass.
- [Validation](validation.json) records source hashes, the pre-existing change
  inventory, scope, and check results. Frozen installation, typecheck,
  **856 frontend tests with 3 skipped**, and production build pass.

## Root-cause boundary and scope

The [historical failing traces](../2026-10-18/README.md) remain the evidence
for the original breaks: a removed presence owner cut exits short, and WebKit
started a CSS opacity transition after Motion's WAAPI fade completed. Today's
source still retains presence ownership and excludes implicit CSS fades on
Motion surfaces. Re-reading the original palette frames confirms opacity
0.999995 at 123 ms becomes 0 at 143 ms on the same node during that historical
animation-type handoff. This failure was not observed in today's replay.

The unchanged browser harnesses run on separately allocated local Vite ports
with synthetic accounts and the mock bridge. Replay using
`node harness/element-presence.mjs` and `node harness/rendering-stability.mjs`.
Today's raw videos stay disposable in `target/flicker-followup-2026-10-21/`;
frame data and this conclusion are retained without duplicate daily recordings.

No production app or user tab was controlled or restarted. No hosted model
test, signing, commit, or release occurred. Native OS-window creation,
installed/packaged WebView behavior, untested popovers, and GPU paint/frame-rate
performance remain unqualified. The existing Rust-upgrade Clippy migration is
outside this replay and untouched.

Raw frame/transcript traces are local generated files, ignored by Git. The
versioned validation receipt and conclusion remain; rerun the listed harnesses
to reproduce the raw observations on another checkout.
