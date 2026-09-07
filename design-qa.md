# Design QA — artifact card edge cleanup

- Source visual truth: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_1fqo02/Screenshot 2026-08-13 at 9.54.25 PM.png`
- Corrected browser capture: `/Users/stan/.codex/visualizations/2026/08/14/019ffe88-e68c-7600-88f5-8ba622d88a0e/artifact-card-borders-removed.png`
- Focused corrected crop: `/Users/stan/.codex/visualizations/2026/08/14/019ffe88-e68c-7600-88f5-8ba622d88a0e/artifact-card-borders-removed-crop.png`
- Source-and-correction comparison: `/Users/stan/.codex/visualizations/2026/08/14/019ffe88-e68c-7600-88f5-8ba622d88a0e/artifact-card-border-comparison.png`
- Source pixels: 590 × 189; implementation viewport: 1440 × 900 CSS px at device scale factor 1; focused correction normalized to 590 × 189.
- State: dark theme, deterministic local mock, Markdown artifact opened in the split artifact workspace.

## Findings and fixes

- [P2] The open artifact card stacked three accent treatments with unrelated widths: full-width top and bottom rules, a 3 px inset left rail, and a 20 px connector extending from the right edge.
  - Fixed by removing the active-only edge decorations and deleting the stale active-state prop from `Conversation` through `ArtifactCard` and `MarkdownDoc`. The card now retains only its normal subtle horizontal dividers.
- Remaining P0/P1/P2 findings: none.

## Required fidelity surfaces

- Typography: unchanged.
- Layout and spacing: unchanged; the artifact still opens in the adjacent workspace and remains keyboard focusable.
- Color: only the existing neutral `border-border-subtle` divider remains on the card.
- Images and assets: none changed.
- Copy and content: unchanged.

## Browser verification

- The opened Markdown artifact still rendered in the split workspace after using its `View` control.
- Computed card borders were 1 px subtle top and bottom, with 0 px left and right borders.
- The card box shadow was `none`, and the conversation wrapper's generated connector content was `none`.
- Browser console errors: none. The only warning was the expected Motion development notice for the system reduced-motion preference.
- Frontend verification: 166 test files and 735 tests passed; typecheck, production build, and `git diff --check` passed.

final result: passed

---

# Design QA — Streamdown reading contrast and data tables

- Source visual truth: `/Users/stan/.codex/generated_images/019ffe88-e68c-7600-88f5-8ba622d88a0e/exec-839eb2f4-2120-49f5-b637-930880b71612.png`
- Browser-rendered implementation: `/Users/stan/.codex/visualizations/2026/08/14/019ffe88-e68c-7600-88f5-8ba622d88a0e/markdown-table-implementation-final.png`
- Full-view comparison: `/Users/stan/.codex/visualizations/2026/08/14/019ffe88-e68c-7600-88f5-8ba622d88a0e/markdown-table-full-comparison.png`
- Focused table comparison: `/Users/stan/.codex/visualizations/2026/08/14/019ffe88-e68c-7600-88f5-8ba622d88a0e/markdown-table-focused-comparison.png`
- Viewport: 946 × 1221 CSS px at device scale 1.
- Source pixels: 1103 × 1426, normalized to 946 × 1221. Implementation pixels: 946 × 1221.
- State: dark theme, 100% text size, Medium overall contrast. The source shows a two-column chat table; the implementation capture uses the same shared Streamdown renderer on a three-column Markdown artifact so the semantic and responsive table contract is exercised without a hosted-model call.

## Findings and fixes

- [P1] Math-enabled Markdown replaced Streamdown's default GitHub-Flavored Markdown plugins, so complete artifact tables rendered as raw pipe text.
  - Fixed by composing `remark-math` with Streamdown's default remark plugins instead of replacing them. Static, streaming, incomplete-stream, and math-enabled table paths now retain semantic `table`, `thead`, and `tbody` output.
- [P2] The selected 40/60 ledger proportions initially applied to every table, collapsing the middle column of a three-column artifact.
  - Fixed by applying 40/60 widths only when the semantic header has exactly two columns; wider tables retain equal fixed tracks.
- [P2] The first responsive pass forced a seven-pixel horizontal overflow at the captured reading width.
  - Fixed by lowering the readable minimum from 34rem to 32rem. The final shell measured 537px client width and 537px scroll width, with no unnecessary scrollbar.
- Remaining P0/P1/P2 findings: none.

## Required fidelity surfaces

- Fonts and typography: Clark's DM Sans hierarchy is preserved. Assistant prose renders at 15px with a 25.8px line height (1.72), using semantic secondary ink for softer reading and primary ink for headings and emphasis. Table copy remains compact at 13px with a 1.55 line height.
- Spacing and layout rhythm: paragraph spacing increases slightly, table cells use 0.7rem × 0.875rem padding, two-column tables use the selected 40/60 ledger split, and wider tables remain readable without collapsed tracks.
- Colors and visual tokens: warm secondary ink replaces glaring primary body copy; the table uses existing elevated, tertiary, strong-border, accent-focus, and alternating-row tokens. No new palette, gradient, or shadow language was introduced.
- Image quality and asset fidelity: no raster assets or new icons are involved. The implementation is native Streamdown/HTML/CSS rather than an image approximation.
- Copy and content: Markdown content remains unchanged. Field labels, values, code spans, headings, paragraphs, and lists retain their semantic source text.

## Browser verification

- A deterministic local mock run created and opened a real Markdown artifact; no hosted model was selected or called.
- The rendered document exposed one accessible `Scrollable table` region containing a semantic table with column headers and cells.
- The final three-column table measured 181px per track before the responsive minimum refinement and then fit its 537px shell without horizontal overflow.
- Chat prose computed to `rgb(198, 196, 191)` at 15px with a 25.8px line height, matching the selected softer reading direction and requested extra line spacing.
- The table focus state remained keyboard-visible with the selected violet-gray outline.
- Browser console errors: none. The only warning was the expected Motion development notice for the system reduced-motion preference.
- Frontend verification: 165 test files / 734 tests passed; typecheck and production build passed.

## Comparison history

1. The selected mock established softer body copy, slightly looser line rhythm, a clear table boundary, roomy rows, and a 40/60 key-value layout.
2. The first real-browser pass exposed raw pipe text in math-enabled artifacts; composing Streamdown's default GFM plugin with math restored the semantic table.
3. The first styled browser capture exposed a collapsed three-column layout; scoping the selected 40/60 split to exactly two columns restored equal wider-table tracks.
4. The next capture exposed a small unnecessary scrollbar; the final responsive minimum removed it while preserving narrow-viewport overflow protection.
5. The final full-view and focused comparisons found no remaining actionable P0/P1/P2 difference in the shared reading and table-rendering contract.

final result: passed

---

# Design QA — SSH config-first remote hosts

- Source visual truth: `/Users/stan/.codex/generated_images/019ffdb3-dbf3-78e2-94e5-36db36c1aaad/exec-1a8584f7-9f7a-4c94-ab0e-4cb3c22d2e14.png`
- Browser-rendered implementation: `/Users/stan/.codex/visualizations/2026/08/14/019ffdb3-dbf3-78e2-94e5-36db36c1aaad/ssh-settings-option-2-exact-state.jpg`
- Full-view comparison: `/Users/stan/.codex/visualizations/2026/08/14/019ffdb3-dbf3-78e2-94e5-36db36c1aaad/ssh-settings-option-2-final-comparison.png`
- Focused comparison: `/Users/stan/.codex/visualizations/2026/08/14/019ffdb3-dbf3-78e2-94e5-36db36c1aaad/ssh-settings-option-2-focused-comparison.png`
- Viewport: 1032 × 593 CSS px at device scale 1.
- Source pixels: 1654 × 951, normalized to 1032 × 593. Implementation pixels: 1032 × 593.
- State: light theme, `gpu-box` selected from SSH config, `/home/ubuntu/projects/clark` entered, successful `Reachable` connection result visible.

## Findings and fixes

- [P1] The original form made users retype SSH destinations already defined in `~/.ssh/config`.
  - Fixed with a native config reader that discovers concrete aliases, follows bounded `Include` files, shows resolved user/hostname details, and keeps manual entry as an explicit fallback.
- [P1] The remote project path had no functional discovery affordance.
  - Fixed by wiring the existing remote-folder browser to the selected SSH config host.
- [P2] Switching setup methods could leave the newly selected method scrolled out of view.
  - Fixed by resetting the dialog content scroll position on mode changes and compacting the modal to keep both steps and actions visible.
- [P2] Connection readiness was not visible in the state-matched design.
  - Fixed with an in-place test action and concise success or failure status.
- Remaining P0/P1/P2 findings: none.

## Required fidelity surfaces

- Typography: Clark's existing interface and monospace type tokens are preserved for labels, aliases, destinations, and paths.
- Layout: the chosen two-path setup, radio-style config list, project-folder step, connection test, and footer action hierarchy match the selected source at the same viewport.
- Color: existing accent, success, surface, border, and ink tokens are reused; no new palette or gradient was introduced.
- Image and asset fidelity: existing Lucide icons are used for server, selection, folder, refresh, success, removal, and close controls; no raster substitutes were introduced.
- Copy and content: the config-first path explains its source, reports discovered hosts, shows resolved destinations, and retains an explicit manual path.

## Comparison history

1. The first browser pass exposed a P2 vertical-scroll issue after switching setup methods; the mode change now resets scroll and the rows and spacing were compacted.
2. The second pass matched the selected visual but lacked the functional folder affordance and same-state success result; the real folder browser, deterministic development probe, and `Reachable` status were added.
3. The final equal-size full-view and focused comparisons found no actionable P0/P1/P2 mismatch. The implementation is slightly taller and adds operational counts and unsaved-state copy; these are acceptable P3 product-status differences.

## Interaction verification

- Selecting a different SSH config alias updates the active destination and project-folder heading.
- Manual mode exposes editable destination and display-name fields, and switching back restores config selection.
- Entering a project folder enables the primary add action.
- Browsing remote folders opens the existing host-scoped folder dialog.
- Testing the selected destination renders the reachable state.
- Adding, editing, removing, and saving hosts exercise the existing persisted-host path.
- Browser console errors: none. The only messages were the expected reduced-motion development warnings.
- Frontend verification: typecheck passed; the focused accessibility and SSH settings tests passed; the earlier full suite passed 733 tests; production build passed.
- Native verification: all 12 SSH parser and command tests passed; Rust formatting check passed.

final result: passed

---

# Design QA — image artifact edge cleanup

- Source visual truth: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_HVRet0/Screenshot 2026-08-13 at 9.54.55 PM.png`
- Browser-rendered implementation: `/Users/stan/.codex/visualizations/2026/08/14/019ffe88-e68c-7600-88f5-8ba622d88a0e/image-artifact-focus-cleanup.png`
- Focused corrected crop: `/Users/stan/.codex/visualizations/2026/08/14/019ffe88-e68c-7600-88f5-8ba622d88a0e/image-artifact-focus-cleanup-crop.png`
- Source-and-correction comparison: `/Users/stan/.codex/visualizations/2026/08/14/019ffe88-e68c-7600-88f5-8ba622d88a0e/image-artifact-focus-comparison.png`
- Source pixels: 623 × 441; implementation viewport: 1440 × 900 CSS px at device scale factor 1; focused correction normalized to 623 × 441.
- State: dark theme, deterministic local mock, inline image artifact with `View` and `Save a Copy` controls.

## Findings and fixes

- [P2] The image artifact exposed the same mixed-width active decoration across its header, preview, and download row, making one card look like several disconnected bordered regions.
  - Fixed at the shared conversation artifact boundary so image, PDF, media, and Markdown cards no longer receive the active accent border, inset rail, or connector stub.
- [P2] The conversation wrapper could still introduce a thick focus ring around the entire multi-layer card.
  - Replaced the outer ring with a quiet semantic highlight on the artifact header, preserving keyboard focus visibility without drawing any card-width borders.
- Remaining P0/P1/P2 findings: none.

## Required fidelity surfaces

- Typography: unchanged.
- Spacing and layout rhythm: card geometry, preview crop, row padding, and control placement are unchanged.
- Colors and tokens: the card retains neutral subtle dividers; keyboard focus uses the existing accent-subtle surface token.
- Image quality and assets: the original SVG preview is unchanged and remains sharp at the rendered size.
- Copy and content: artifact title, type, `View`, and `Save a Copy` labels are unchanged.

## Browser verification

- The inline SVG, `View`, `Save a Copy`, and neighboring PDF artifact all rendered in the real conversation surface.
- The image card showed no accent rail, connector, shadow, or side borders in the normalized comparison.
- The card retained only 1 px neutral top and bottom dividers with 0 px left and right borders.
- Browser console errors: none. The only warning was the expected Motion development notice for the system reduced-motion preference.
- Frontend verification: 166 test files and 736 tests passed; typecheck, production build, and `git diff --check` passed.

final result: passed

---

# Design QA — Canvas Drop artifact experience

- Selected visual truth: `/Users/stan/.codex/generated_images/019ffe88-e68c-7600-88f5-8ba622d88a0e/exec-b395c507-55ba-4c3e-8ff2-29c4cbdeb106.png`
- Normalized source: `/Users/stan/.codex/visualizations/2026/08/14/019ffe88-e68c-7600-88f5-8ba622d88a0e/canvas-drop-source-normalized.png`
- Browser-rendered implementation: `/Users/stan/.codex/visualizations/2026/08/14/019ffe88-e68c-7600-88f5-8ba622d88a0e/canvas-drop-implementation-final.png`
- Compact verification: `/Users/stan/.codex/visualizations/2026/08/14/019ffe88-e68c-7600-88f5-8ba622d88a0e/canvas-drop-compact.png`
- Full comparison: `/Users/stan/.codex/visualizations/2026/08/14/019ffe88-e68c-7600-88f5-8ba622d88a0e/canvas-drop-comparison-final.png`
- Focused comparison: `/Users/stan/.codex/visualizations/2026/08/14/019ffe88-e68c-7600-88f5-8ba622d88a0e/canvas-drop-focused-comparison-final.png`
- Source pixels: 1487 × 1058, normalized to 1440 × 1024; implementation viewport: 1440 × 1024 CSS px at device scale factor 1; compact viewport: 1024 × 768.
- State: dark theme, deterministic local mock, inline SVG artifact in the canonical conversation with the artifact workspace closed for the primary comparison.

## Findings and fixes

- [P2] Visual artifacts were presented as conventional stacked file rows, so the generated work had little hierarchy and its controls felt detached from the preview.
  - Replaced the image and video path with a borderless Canvas Drop: compact metadata, a rounded hero canvas, and a floating two-action dock that keeps the artifact itself visually primary.
- [P2] The first implementation repeated the raw filename as the dominant label and made the metadata read like storage plumbing.
  - Added a human display title, retained the file extension as scannable technical context, and separated the source filename with the existing accent token.
- [P2] A generated timestamp would have implied data the current `Artifact` contract does not provide.
  - Used the truthful status `Ready to review` while preserving the selected concept's compact metadata rhythm.
- Remaining P0/P1/P2 findings: none.

## Required fidelity surfaces

- Typography: the existing DM Sans product typography is retained; the preview itself remains the real SVG asset rather than reconstructed decorative markup.
- Spacing and layout: metadata, hero canvas, and overlapping action dock match the selected concept. The canonical 50rem conversation measure is intentionally narrower than the concept's open showcase layout so it continues to coexist with the real sidebar and artifact workspace.
- Colors and tokens: the implementation reuses the existing accent, canvas, foreground, muted-foreground, and border tokens; no parallel palette was introduced.
- Image quality and assets: the real generated SVG remains sharp, uncropped, and centered inside the dark canvas at desktop and compact widths.
- Copy and content: `Open workspace` and `Save a copy` express the two primary artifact actions without adding a third menu or unverified creation time.

## Browser verification

- `Open workspace` opened the real artifact workspace and selected `artifact-preview.svg`; closing it returned to the uninterrupted conversation state.
- At 1024 × 768, document width and scroll width both measured 1024 px, the 704 px hero remained inside the viewport, and both floating controls were fully visible.
- The final 1440 × 1024 comparison preserved the selected metadata strip, large blue/teal preview, and floating two-button action dock while keeping neighboring real artifacts intact.
- Browser console errors: none. The only warning was the expected Motion development notice for the system reduced-motion preference.
- Frontend verification: 166 test files and 737 tests passed; typecheck, production build, and `git diff --check` passed.

final result: passed
