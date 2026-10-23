# Rendering flicker replay — 2026-10-22

No current flicker or new rendering contract break was reproduced at foundation
`fde7f2db38127b9705eb43e72b399f7f4d3747e8`. All 32 sources covered by
yesterday's receipt match, and there are no committed frontend/harness changes
since that capture. Existing mobile-project, dependency, and other dirty work
were preserved. No app or harness patch was made.

## Fresh evidence

- [Dialog frames](frames.json): 2,041 animation-frame observations across 20
  first-mount, settled-exit, and 50 ms close/reopen cases. Palette, new session,
  Settings, MCP, and SSH pass in Chromium/WebKit with normal/reduced motion.
  Entry reversals, competing CSS opacity transitions, rapid node replacements,
  settled displacement, and page errors are zero. Every exit has intermediate
  frames; the minimum is three.
- [Transcript results](transcript-results.json): four engine/motion
  configurations with 12 commands each and 57–72 sampled command frames.
  Overlap, duplicates, replacements, status/finish height shifts, and page errors
  are zero. History, scroll ownership, disclosure, short-answer geometry, and
  Markdown identity checks pass.
- [Validation](validation.json) retains source hashes, the preservation
  inventory, scope, and results. Only the two rendering harnesses were rerun.

The [prior full frontend gates](../2026-10-21/validation.json) passed frozen
installation, typecheck, 856 tests (3 skipped), and production build. They were
not rerun today: the monitored sources and all existing frontend/harness dirty
files match that receipt, there are no intervening frontend/harness commits,
and no code patch was made. This is a source comparison, not a new full-suite
result.

## Contract boundary and limitations

The [historical failure evidence](../2026-10-18/README.md) identifies the original
breaks: destroyed dialog presence ownership and WebKit starting a second CSS
opacity fade after Motion completed its WAAPI fade. Re-reading the retained
palette frames confirms opacity 0.999995 at 123 ms becomes 0 at 143 ms on the
same node at that handoff. Today's source still contains both fixes; today's
replay did not reproduce those failures or justify another rendering patch.

Unchanged harnesses start their own local Vite servers on allocated ports using
synthetic accounts and the mock bridge. Replay with
`node harness/element-presence.mjs` and `node harness/rendering-stability.mjs`.
Raw recordings remain disposable in `target/flicker-followup-2026-10-22/`;
complete frame data and this conclusion are retained without duplicate videos.

No production app or user tab was controlled or restarted. No hosted-model
test, signing, commit, or release occurred. Native OS-window creation,
installed/packaged WebView behavior, untested popovers, and GPU paint/frame-rate
performance remain unverified. The separate Rust-upgrade Clippy work is
unchanged and outside this replay.
