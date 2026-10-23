# Rendering flicker follow-up — 2026-10-19

No current flicker was reproduced in the isolated browser fixtures. The prior
presence and opacity-ownership fixes remain intact while the working tree
contains the uncommitted React 19.3.1 / Vite 8.3.4 / Tauri dependency candidate.
No application or harness source was changed for this follow-up.

## Contract review before changes

All 22 surviving rendering/harness source hashes in the October 18 validation
receipt match the current files. The retained failure frames establish the
first broken boundaries without restoring old code into this shared checkout:

1. The original workspace removed a lazy dialog's presence owner at close.
   The dialogs detached in 3–14 ms without an intermediate exit frame; rapid
   reopening created a different node. The stable outer presence owners and
   propagated exits are still present.
2. After that repair, WebKit with reduced motion handed a finished Motion
   opacity animation to an unintended CSS opacity transition. In the retained
   palette trace, the same node goes from opacity 0.999995 at 123 ms (WAAPI
   `Animation`, inline opacity 0) to opacity 0 at 143 ms (`CSSTransition` for
   opacity, inline opacity 1). Its position and identity did not change. The
   current CSS still limits nonzero transition durations to explicit transition
   utilities, so Motion surfaces do not receive that competing fade.

These are historical reproductions in the
[October 18 evidence](../2026-10-18/README.md), not newly observed failures on
today's source. The current fixtures below did not find another broken boundary,
so no speculative rendering patch was applied.

## Fresh evidence

- Frame samples (local `frames.json`): 2,037 samples across 20 cases, testing first
  mount, settled exit, and close/reopen after 50 ms. Command palette, new
  session, Settings, MCP, and SSH dialogs pass in Chromium and WebKit with both
  motion preferences. There are zero opacity reversals, competing CSS opacity
  transitions, replaced nodes on rapid reopening, settled displacement, or page
  errors. Every case retains intermediate exit frames; the minimum is three.
- Local recordings (`chromium-no-preference.webm`, `webkit-reduce.webm`) and
  screenshot (`session.png`) use a synthetic local account. These generated
  media files are ignored by Git; the frame samples and validation hashes remain
  versioned.
  Composer text is preserved across every transition. Duplicate raw recording
  copies were removed after comparing their hashes to the retained files.
- Transcript results (local `transcript-results.json`): four engine/motion
  configurations, each retaining 12 visible command identities and titles.
  Across 59–72 sampled command frames per configuration there is zero overlap,
  replacement, duplicate rows, or status/finish height shift. History, scroll
  ownership, disclosure, short replies, and Markdown retention pass too.
- [Validation](validation.json) retains source/artifact hashes, exact counts,
  and the inventory of existing changes preserved before documentation.

Frozen frontend installation, typecheck, **848 tests passed with 3 skipped**,
and the production build pass on the current dependency candidate.

Replay from the foundation checkout:

```sh
OUTPUT="$PWD/target/flicker-followup/element-presence" node harness/element-presence.mjs
node harness/rendering-stability.mjs
pnpm --dir app install --frozen-lockfile
pnpm --dir app typecheck
pnpm --dir app test
pnpm --dir app build
```

## Scope and remaining uncertainty

Each browser harness starts its own local Vite server on an allocated port and
uses synthetic local state with the mock bridge. The screenshot's phone-access
warning belongs to that fixture account; it says nothing about the user's real
Android/desktop connection. No active production app or user browser tab was
controlled or restarted. No provider/model calls, signing, commits, or release
publication occurred.

These measurements cover DOM presence, opacity, and geometry. They do not
establish a frame-rate benchmark, native OS-window creation, a packaged WebView
journey, or every untested popover. The pending Rust compiler/Clippy migration
belongs to the separate dependency-upgrade task and remains untouched.

Raw frame/transcript traces are local generated files, ignored by Git. The
versioned validation receipt and conclusion remain; rerun the listed harnesses
to reproduce the raw observations on another checkout.
