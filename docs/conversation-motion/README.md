# Conversation motion review — 2026-10-17

[Open the before/after review](index.html), or play the
[updated video](after/review.mp4) / [GIF](after/review.gif).
The review page offers side-by-side playback, pause, and half speed.

## What changed and why

- Image thumbnails gently lift 2px and scale to 1.015 on hover, then compress
  to 0.99 on press. User attachments, Markdown images, generated images, and
  artifact-workspace images share this interaction.
- The image viewer fades its scrim and softly reveals its image and toolbar.
  The baseline capture exposed a functional bug: a message-row transform
  confined the fixed viewer to the row. A body portal now keeps it full-screen.
  Keyboard focus stays within the viewer, the exiting viewer becomes inert,
  and focus returns to the thumbnail without moving the transcript.
- Reply placeholders fade as they are consumed, outside document flow.
  The answer's height still settles in one step; streamed Markdown and code
  blocks keep their existing stable renderer.
- Attachment chips enter and leave with the shared small-rise vocabulary.
  Remaining chips animate position without distorting their contents, and
  removing the final chip now fades the rail instead of unmounting it abruptly.
- Reduced motion removes image gestures and spatial reveals while retaining
  short opacity cues. All motion uses the existing shared timing presets.

## Recorded evaluation

The captures use the actual React app with synthetic local store snapshots.
No user conversations, credentials, hosted-model calls, or release deployment
were used. `before/` was recorded before the implementation; `after/` records
the final implementation. The updated sequence adds attachment-removal and
Markdown/generated-image acceptance after the shared hover/viewer/streaming/
notice sequence. Startup times and video frame counts are not performance
measurements. The contact sheet and screenshots were visually reviewed.

```sh
node harness/conversation-motion.mjs
node harness/rendering-stability.mjs
node harness/workspace-motion.mjs
node harness/artifact-delivery-smoke.mjs
```

The motion harness runs Chromium and WebKit with both normal and reduced
motion. It records a normal-motion Chromium video/GIF and retains the four
[geometry/focus/presence receipts](after/receipts.json). `--baseline` is for
capturing the source before an implementation change, not for recreating an
older implementation after edits. Raw intermediate videos stay under `target/`.

| Check | Result |
| --- | --- |
| Motion matrix | 4/4 configurations passed; no page errors |
| Hover | 318px thumbnail grows to 322.77px; reduced motion stays at 318px |
| Viewer | Full 1280×900 viewport; focus trapped and restored in all four runs |
| Exit presence | Viewer and final attachment rail visibly fade before removal |
| Transcript movement from viewer | 0px in all four runs |
| Rendering stability | 4/4 configurations passed: 60 saved receipts settled, 0px reading/disclosure/scrollback displacement, retained code-fence node, zero intermediate completion-height frames |
| Workspace navigation | 4/4 configurations passed |
| Artifact delivery | 6/6 existing image/PDF/download checks passed |
| Focused motion/message suite | 43 tests passed |
| Frontend | Frozen install, typecheck, and production build passed |
| Foundation Rust | Strict Clippy, 802 nextest tests (8 skipped), and WASM check passed; nextest `8188b912-bc33-4eee-ae29-8a8c539f27e7` |

## Shared-checkout limits

The full frontend suite initially passed 844 tests (3 skipped). A later run
after concurrent authentication edits passed 843 and failed the unrelated
`sessionStore.cloudLifecycle.spec.ts` reconnect-state assertion (`offline`
versus `reconnect_required`). Those files were not changed for this work.
The production build and focused motion tests passed after the final UI edits.

Repository-wide `cargo fmt --all --check` is blocked by the missing unrelated
`src-tauri/src/product/tests.rs`. The canonical unsigned launcher compiled in
47.60 seconds and started this checkout's `target/debug/clark-code`; native
motion interaction was not qualified. The computer-use app selector resolved
an older installed QA app and could not bind the raw development executable.
Browser WebKit acceptance is distinct from native packaged-product or Windows/
Linux qualification. No signing identity or Keychain access was used.


## Combined release verification — 2026-10-17

The concurrent authentication work is now complete in the combined source.
The full frontend suite passes 849 tests (3 skipped), including the revised
explicit offline/reconnect contract. Typecheck, neutral and branded production
builds, repository Rust formatting, strict foundation Clippy, 802 scoped Rust
tests (8 skipped), 155 native-host tests, and the WASM check pass. The previously
missing `src-tauri/src/product/tests.rs` is present and its ownership/deadlock
regressions pass. The historical failure above describes the intermediate
shared checkout, not the combined release candidate. Native packaged motion
acceptance remains separate from these local checks.
