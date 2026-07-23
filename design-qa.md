# Goal UX design QA

- Source visual truth: `<local QA artifacts>/goal-ux/1-Photo-1.jpg`
- Implementation screenshot: `/tmp/clark-goal-ux-390x844.png`
- Viewport: 390 x 844 CSS pixels
- State: dark theme, blocked goal, collapsed work receipt, 24-file change receipt

## Full-view comparison evidence

The source and implementation were opened together in one comparison pass. Both expose the same primary hierarchy: user request, a quiet `Worked for 43s` disclosure, terminal answer, compact file-change receipt, blocked-goal receipt, and composer. Clark retains its existing desktop side rail, typography, warm-graphite tokens, and composer instead of copying the reference chrome.

## Required fidelity surfaces

- Fonts and typography: Clark correctly uses its existing DM Sans and mono receipt styles. The work label, terminal answer, and compact metadata preserve the source hierarchy and remain readable at the mobile viewport.
- Spacing and layout rhythm: the disclosure divider, open breathing room, bottom receipts, checkout context, and composer match the source's vertical structure. No horizontal overflow occurs at 390 pixels.
- Colors and visual tokens: the source's quiet dark surfaces and muted metadata are mapped to Clark's existing dark tokens; success additions and danger deletions remain semantically distinct.
- Image quality and asset fidelity: the target contains no product imagery or decorative raster assets. Standard controls use the repository's existing Lucide icon system; no placeholder or handcrafted asset substitutes were introduced.
- Copy and content: the simulation uses realistic goal completion/blocker language and preserves the compact source labels (`Worked for`, file count, and `Goal blocked`).

The full-view comparison kept all important type, disclosure, and chip details readable, so a separate focused crop was not needed. The goal and change popovers were additionally opened and visually inspected in-browser.

## Interaction and runtime evidence

- Expanded and collapsed the `Worked for 43s` work receipt.
- Opened and closed the blocked-goal detail receipt.
- Opened and closed the 24-file change receipt.
- Verified the composer remains visible and the body is exactly 390 x 844 with no horizontal overflow.
- Checked browser console errors: none.

## Findings

No actionable P0, P1, or P2 findings remain.

The retained desktop side rail and Clark-specific composer are intentional product constraints, not fidelity defects: this work ports the interaction semantics into Clark Desktop rather than cloning the reference mobile shell.

## Comparison history

1. Initial 390-pixel pass: the second receipt extended past the conversation column. Fixed the narrow-viewport rail padding and chip gaps. Post-fix evidence shows both receipts fully visible.
2. Expanded-work pass: collapsing a long receipt could leave the `Jump to latest` state visually stale. Fixed the conversation scroll state to settle immediately when the control is used. Post-fix evidence shows the compact transcript and composer without the stale control.

## Follow-up polish

- P3: consider an even more compact receipt-label variant if the desktop side rail gains additional width at a future breakpoint.

final result: passed

---

# Complete UI surface polish audit

- Audit directory: `<local QA artifacts>/all-ui-audit`
- Broken-state screenshot: `<local QA artifacts>/all-ui-audit/18-account-menu.png`
- Fixed-state screenshot: `<local QA artifacts>/all-ui-audit/20-account-menu-fixed.png`
- Before/after comparison: `<local QA artifacts>/all-ui-audit/account-menu-before-after.png`
- Primary viewport: 1434 × 1230 CSS pixels at device scale factor 1
- Responsive stress viewport: 700 × 700 CSS pixels
- States: dark and light themes; expanded and collapsed sidebars; chat, settings, panels, menus, and popovers

## Audited surfaces

1. Chat shell: header, transcript, work checklist, tool rows, research receipt, diff summary, checkout context, composer, and fixed-bottom controls.
2. Navigation: collapsed and expanded sidebars, project rail, active conversation, conversation search, project actions, and account entry points.
3. Settings: General, Account, Project, Command policy, Integrations, and About.
4. Secondary panels: Memory, Terminal, Changes, and Share.
5. Composer popovers: attachment picker, approval policy, and model picker.
6. Responsive and theme variants: narrow chat, narrow settings, and light-theme chat.

## Required visual-quality surfaces

- Fonts and typography: the existing DM Sans and JetBrains Mono hierarchy remains consistent across app chrome, chat, receipts, settings, and popovers. No unintended wrapping, clipping, or hierarchy regressions were observed.
- Spacing and layout rhythm: primary rails, panel gutters, settings sections, transcript cadence, composer spacing, and popover offsets remain aligned to the existing design system. The collapsed account menu is now offset 8 pixels to the right of the rail and 8 pixels above the viewport edge.
- Colors and visual tokens: both themes preserve the existing graphite surfaces, muted borders, semantic diff colors, and violet focus/selection treatment. No hard-coded replacement palette was introduced.
- Image quality and asset fidelity: the audit uses the repository's existing Clark mark and Lucide icon system. No placeholder, generated, or approximate assets were introduced.
- Copy and content: labels, statuses, source counts, project names, settings descriptions, and account content remained unchanged by the positioning fix.

## Finding and resolution

1. P1: the collapsed-sidebar account menu used the topbar placement rule (`right: 0; top: 100%`), rendering its 288 × 348 pixel surface at x = -250.5 and y = 1230, entirely outside the 1434 × 1230 viewport.
   - Fix: introduced a dedicated collapsed-rail placement that opens to the right and aligns to the bottom of the trigger.
   - Post-fix evidence: the menu renders at x = 45.5 and y = 873.67, with right = 333.5 and bottom = 1222. Every edge remains inside the viewport with an 8-pixel bottom inset.
   - Interaction evidence: the menu opens and closes through the same Account button; the expanded-sidebar and topbar placement variants remain unchanged.

## QA result

The same-size before/after composite was inspected as a single comparison image. The fixed menu is fully visible, uses the existing popover styling, avoids covering the composer, and stays visually anchored to the collapsed account avatar. The remaining 22 representative states showed no actionable P0, P1, or P2 visual defects.

final result: passed

---

# Research receipt chrome and spacing design QA

- Source visual truth:
  - `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_txVrPS/Screenshot 2026-07-22 at 10.43.18 PM.png`
  - `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_j1bnx8/Screenshot 2026-07-22 at 10.44.04 PM.png`
- Implementation screenshot: `<local QA artifacts>/research-receipt-cleanup/implementation-full-700x700.png`
- Focused implementation crop: `<local QA artifacts>/research-receipt-cleanup/implementation-focused-635x119.png`
- Side-by-side comparison: `<local QA artifacts>/research-receipt-cleanup/comparison.png`
- Viewport: 700 × 700 CSS pixels at device scale factor 1
- Source and focused implementation dimensions: 635 × 119 pixels each; no density scaling
- State: dark theme, collapsed research receipt, completed read/edit/research work block

## Full-view and focused comparison evidence

The focused comparison puts the supplied cramped state and the revised browser render in one 1268 × 119 image. The earlier receipt used a 2-pixel violet cap, a stronger border, and a shadow that combined into a heavy rounded rim. It also began 8 pixels below the edit row. The revised receipt uses one subtle 1-pixel token border, no decorative cap, no shadow, and a 16-pixel tool-to-research gap.

The full-view browser capture confirms the quieter receipt still reads as a distinct interactive result and remains aligned with the 604-pixel chat rail at the narrow viewport.

## Required fidelity surfaces

- Fonts and typography: existing DM Sans and JetBrains Mono faces, sizes, weights, line heights, wrapping, and truncation are unchanged.
- Spacing and layout rhythm: the edit-to-research transition increases from 8 to 16 pixels. Internal receipt padding and the surrounding 20-pixel top-level chat cadence remain unchanged.
- Colors and visual tokens: the custom violet cap and soft shadow are removed. The receipt now uses the existing subtle border token and a quieter secondary background.
- Image quality and asset fidelity: the Clark mark and existing icon assets are unchanged; no new imagery or replacement assets were introduced.
- Copy and content: research title, status, source count, query, and action copy are unchanged.

## Interaction and runtime evidence

- Opened and closed the revised Clark Cloud Agent receipt at the 700-pixel viewport.
- Verified the expanded findings, sources, links, and collapse control still render without horizontal overflow.
- Measured 16 pixels between the edit row and receipt, a 1-pixel top border, no box shadow, and zero document overflow.
- Frontend typecheck, 336 tests across 77 files, and production build passed.

## Findings and comparison history

1. Initial P2: the receipt's violet cap, stronger border, and shadow produced the sloppy double-rim chrome shown in the source crop.
   - Fix: removed the cap and shadow, reduced the border to the subtle token, and lowered the background emphasis.
   - Post-fix evidence: the focused comparison shows one quiet neutral outline with no accent stripe or shadow seam.
2. Initial P2: the 8-pixel gap between the edit row and receipt made separate work phases feel crowded.
   - Fix: increased the transition to 16 pixels.
   - Post-fix evidence: browser measurement and the same-size comparison both show the doubled gap without loosening the receipt's internal layout.

No actionable P0, P1, or P2 findings remain.

## Follow-up polish

No P3 follow-up is needed for this focused cleanup.

final result: passed

---

# Composer proportion adjustment design QA

- Source visual truth: `/Users/stan/Desktop/Screenshot 2026-07-22 at 10.24.50 PM.png`
- Implementation screenshot: `<local QA artifacts>/composer-resize/design-qa-implementation.png`
- Full implementation screenshot: `<local QA artifacts>/composer-resize/design-qa-implementation-full.png`
- Side-by-side comparison: `<local QA artifacts>/composer-resize/design-qa-comparison.png`
- Viewport: 956 × 600 CSS pixels at device scale factor 1; source and focused implementation crops are both 956 × 173 pixels
- State: dark theme, active mock conversation, collapsed sidebar, 110% text size, idle composer

## Full-view and focused comparison evidence

The source and implementation crops were joined into one 1912 × 173 comparison image before review. The source composer is approximately 880 CSS pixels wide at the matching 110% text scale, making the requested 70% target 616 pixels. The implementation measures exactly 616 pixels. Its measured CSS height is 106.31 pixels; subtracting the 14 pixels of added vertical padding gives the prior 92.31-pixel layout, so the revised card is 15.2% taller.

The component crop is already a focused comparison: the checkout context, textarea, approval and execution controls, model picker, and send button remain readable at the new proportions. A second detail crop was not needed.

## Required fidelity surfaces

- Fonts and typography: the existing DM Sans and JetBrains Mono system is unchanged. At the matching 110% text setting, placeholder and control text do not wrap or truncate unexpectedly.
- Spacing and layout rhythm: the composer, checkout context, autocomplete, queued messages, and readiness hint share the new 35rem rail. Conversation content keeps its existing 50rem reading rail. The 17-pixel top and bottom padding produces the requested height increase without shifting the control hierarchy.
- Colors and visual tokens: the warm graphite surface, violet focus treatment, borders, and semantic control colors are unchanged.
- Image quality and asset fidelity: the target contains no product imagery or custom decorative assets. Existing repository icons are unchanged.
- Copy and content: no app copy changed. The mock preview uses a different approval policy and model than the source screenshot, which is expected fixture state and was excluded from the sizing comparison.

## Interaction and runtime evidence

- Entered and submitted a message through the composer to create the active mock-conversation state.
- Verified the textarea, approval control, execution control, model picker, and send button fit without overflow.
- Checked browser console errors: none.
- Frontend typecheck, 336 tests across 77 files, and production build passed.

## Findings

No actionable P0, P1, or P2 findings remain. The measured width matches the 70% target exactly at the reference text scale, and the card height increases by 15.2%.

## Comparison history

1. Initial pass at 100% text size measured the new composer at 560 × 103.75 pixels.
2. The verification state was changed to the source screenshot's 110% text scale. The post-change measurement is 616 × 106.31 pixels, matching the requested width and height ratios. No implementation fix was needed after the visual comparison.

## Follow-up polish

No P3 follow-up is needed for this sizing-only change.

final result: passed

---

# Chat column and composer width design QA

- Source visual truth: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_BLgL3e/Screenshot 2026-07-20 at 1.33.52 PM.png`
- Source target crop: `/tmp/clark-desktop-design-qa/reference-chat.png`
- Implementation screenshot: `/tmp/clark-desktop-design-qa/implementation-final-neutral.png`
- Full-view comparison: `/tmp/clark-desktop-design-qa/final-comparison.png`
- Focused composer comparison: `/tmp/clark-desktop-design-qa/composer-comparison-final.png`
- Viewport: 1600 × 900 desktop
- State: light theme, completed mock conversation, Default text size, composer visible

## Findings

No actionable P0, P1, or P2 differences remain for the requested chat-column and composer-width change.

- Fonts and typography: Clark keeps its existing DM Sans, Newsreader, and JetBrains Mono system. Text remains readable at Compact, Default, and Large. Matching the reference typography was not part of the requested change.
- Spacing and layout rhythm: the reference conversation rail occupies approximately 50% of its app window. Clark now uses an 800px rail at the 1600px comparison viewport, with the conversation, composer, context bar, queued messages, goal rail, and start screen sharing the same centered edge. The narrow viewport fallback remains fluid because the rail has `width: 100%` within existing page padding.
- Colors and visual tokens: Clark's existing warm light palette and dark-mode tokens are unchanged. The comparison uses light mode only to make the reference proportions easier to judge.
- Image quality and asset fidelity: this change introduces no images, illustrations, logos, or replacement assets.
- Copy and content: existing Clark copy and the mock conversation remain unchanged. The reference and implementation contain different conversation text, so copy was excluded from fidelity scoring.

## Interaction verification

- Default: 800px rail (`50rem`).
- Command-plus: Large preset and 880px rail.
- Command-minus: Compact preset and 720px rail.
- Command-zero: reset to Default and 800px rail.
- Composer submission completed through the mock provider and rendered a full conversation on the same rail.
- Browser console errors checked: none.

## Comparison history

1. Initial pass used a 48rem Default rail (768px / 48% of the viewport). The full comparison showed it remained about 5% narrower than the reference.
2. The base rail was increased to 50rem while keeping the existing 90% / 100% / 110% text-size scaling.
3. The final full-view and focused comparisons show the Default composer and conversation at the same approximate 50% window proportion as the reference, with aligned supporting surfaces.

Focused comparison was required because the supplied reference is a low-resolution side-by-side screenshot and composer details are too small to judge reliably in the full view alone.

final result: passed

---

# Chat vertical rhythm design QA

- Source visual truth: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_OOJDpT/Screenshot 2026-07-22 at 10.34.35 PM.png`
- Implementation screenshot: `<local QA artifacts>/chat-spacing/implementation-crop.png`
- Full implementation screenshot: `<local QA artifacts>/chat-spacing/implementation-full.png`
- Side-by-side comparison: `<local QA artifacts>/chat-spacing/comparison.png`
- Viewport: 1434 × 1230 CSS pixels at device scale factor 1; source and normalized implementation crops are both 965 × 517 pixels
- State: dark theme, active mock conversation, collapsed sidebar, 110% text size, collapsed research receipt

## Full-view and focused comparison evidence

The source and implementation crops were normalized to the same 965 × 517 pixels and joined into one 1930 × 517 comparison image. The screenshot exposed three cramped transitions: ordinary timeline blocks were 16 pixels apart, tool-run boundaries were compressed to 12 pixels, and adjacent tool rows had no gap. The implementation measures 20 pixels between every top-level chat block and 4 pixels between tool rows. This is enough to separate each thought and work receipt without turning the transcript into a loose card feed.

The normalized crop is already focused on the complete visible transcript stack, so a second detail crop was unnecessary.

## Required fidelity surfaces

- Fonts and typography: existing DM Sans and JetBrains Mono typography, weights, line heights, wrapping, and antialiasing are unchanged.
- Spacing and layout rhythm: top-level chat spacing increases from 16 to 20 pixels. Removing the work block's negative vertical margin restores 20-pixel transitions around tool runs, and the new 4-pixel internal gap separates adjacent tool rows.
- Colors and visual tokens: backgrounds, borders, violet research accent, and semantic diff colors are unchanged.
- Image quality and asset fidelity: the target contains no product imagery or custom decorative assets. Existing repository icons remain unchanged.
- Copy and content: no app copy, ordering, grouping, or transcript content changed.

## Interaction and runtime evidence

- Opened and closed the Clark Cloud Agent research receipt after the spacing change.
- Verified the transcript, work checklist, tool rows, research card, edit summary, and final answer remain aligned within the 880-pixel chat rail.
- Checked browser console errors: none.
- Frontend typecheck, 336 tests across 77 files, and production build passed.

## Findings

No actionable P0, P1, or P2 findings remain. The transcript now has a consistent, readable vertical cadence while keeping related tool work visually grouped.

## Comparison history

1. Initial source pass measured 16-pixel timeline gaps, 12-pixel tool-run boundaries, and 0-pixel spacing between adjacent tool rows.
2. The post-change comparison measures 20-pixel timeline and tool-run boundaries plus 4-pixel tool-row spacing. The normalized side-by-side shows the final answer moving 37 pixels lower across the complete stack, distributing the extra room instead of adding one large isolated gap.

## Follow-up polish

No P3 follow-up is needed for this spacing-only change.

final result: passed
