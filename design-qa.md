# Goal UX design QA

- Source visual truth: `/tmp/codex-remote-attachments/019f7b3d-f550-7ee3-b77a-787a77450191/D0C40645-6D73-4BE7-8BF4-3E0E5EE4D7B6/1-Photo-1.jpg`
- Implementation screenshot: `/tmp/clark-goal-ux-390x844.png`
- Viewport: 390 x 844 CSS pixels
- State: dark theme, blocked goal, collapsed work receipt, 24-file change receipt

## Full-view comparison evidence

The source and implementation were opened together in one comparison pass. Both expose the same primary hierarchy: user request, a quiet `Worked for 43s` disclosure, terminal answer, compact file-change receipt, blocked-goal receipt, and composer. Clark retains its existing desktop side rail, typography, warm-graphite tokens, and composer instead of copying Codex chrome.

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

The retained desktop side rail and Clark-specific composer are intentional product constraints, not fidelity defects: this work ports the interaction semantics into Clark Desktop rather than cloning the Codex mobile shell.

## Comparison history

1. Initial 390-pixel pass: the second receipt extended past the conversation column. Fixed the narrow-viewport rail padding and chip gaps. Post-fix evidence shows both receipts fully visible.
2. Expanded-work pass: collapsing a long receipt could leave the `Jump to latest` state visually stale. Fixed the conversation scroll state to settle immediately when the control is used. Post-fix evidence shows the compact transcript and composer without the stale control.

## Follow-up polish

- P3: consider an even more compact receipt-label variant if the desktop side rail gains additional width at a future breakpoint.

final result: passed

---

# Chat column and composer width design QA

- Source visual truth: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_BLgL3e/Screenshot 2026-07-20 at 1.33.52 PM.png`
- Source target crop: `/tmp/clark-desktop-design-qa/reference-codex.png`
- Implementation screenshot: `/tmp/clark-desktop-design-qa/implementation-final-neutral.png`
- Full-view comparison: `/tmp/clark-desktop-design-qa/final-comparison.png`
- Focused composer comparison: `/tmp/clark-desktop-design-qa/composer-comparison-final.png`
- Viewport: 1600 × 900 desktop
- State: light theme, completed mock conversation, Default text size, composer visible

## Findings

No actionable P0, P1, or P2 differences remain for the requested chat-column and composer-width change.

- Fonts and typography: Clark keeps its existing DM Sans, Newsreader, and JetBrains Mono system. Text remains readable at Compact, Default, and Large. Matching Codex typography was not part of the requested change.
- Spacing and layout rhythm: the reference Codex conversation rail occupies approximately 50% of its app window. Clark now uses an 800px rail at the 1600px comparison viewport, with the conversation, composer, context bar, queued messages, goal rail, and start screen sharing the same centered edge. The narrow viewport fallback remains fluid because the rail has `width: 100%` within existing page padding.
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

1. Initial pass used a 48rem Default rail (768px / 48% of the viewport). The full comparison showed it remained about 5% narrower than the Codex reference.
2. The base rail was increased to 50rem while keeping the existing 90% / 100% / 110% text-size scaling.
3. The final full-view and focused comparisons show the Default composer and conversation at the same approximate 50% window proportion as the reference, with aligned supporting surfaces.

Focused comparison was required because the supplied reference is a low-resolution side-by-side screenshot and composer details are too small to judge reliably in the full view alone.

final result: passed
