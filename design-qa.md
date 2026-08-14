# Design QA — compact Spec sidebar

- Source visual truth: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_QKK1Kh/Screenshot 2026-08-10 at 9.45.08 PM.png`
- Implementation screenshot: `/Users/stan/.codex/visualizations/2026/08/10/019fed7c-690d-7241-a304-b500474e5b77/spec-sidebar-compact-200-matched.png`
- Focused implementation crop: `/Users/stan/.codex/visualizations/2026/08/10/019fed7c-690d-7241-a304-b500474e5b77/spec-sidebar-compact-200-matched-crop.png`
- Combined comparison: `/Users/stan/.codex/visualizations/2026/08/10/019fed7c-690d-7241-a304-b500474e5b77/spec-sidebar-density-comparison-matched.png`
- Viewport: 804 × 516 CSS px.
- Source pixels: 434 × 320; implementation pixels: 804 × 516; focused crop: 434 × 320.
- Density normalization: no resampling; the implementation was captured at Clark's visible 200% text-size setting and cropped to the source dimensions.
- State: light theme, Spec active, New spec visible, and one customer-segmentation Spec conversation visible.

## Findings and fixes

- [P1] Vertical density — the source used approximately 72px for the Spec row and 64px for New spec at the enlarged reading size.
  - Fixed with 44px and 40px control heights that do not scale with the text-size preference.
- [P1] Horizontal alignment — source icons and labels depended on rem-scaled gaps and padding, producing inconsistent columns.
  - Fixed with one fixed-pixel icon/label rhythm: 22px specialist icon, 18px create icon, and 10px gaps; nested conversations use a deliberate single indent.
- [P2] Redundant model badge — `Free` consumed the far edge of the primary Spec row even though the Spec model is automatic.
  - Fixed by suppressing the badge for Spec while preserving badges for other specialist products.
- Remaining P0/P1/P2 findings: none.

## Required fidelity surfaces

- Fonts and typography: existing Clark font, weight, color, and 200% user text setting preserved; `leading-none` keeps enlarged labels optically centered in fixed-height controls.
- Spacing and layout rhythm: Spec is 44px, New spec is 40px, and the saved conversation is 36px; all three share stable vertical centers and intentional indentation.
- Colors and visual tokens: existing selected violet, paper background, borders, and ink tokens preserved.
- Image quality and asset fidelity: existing product-provided icon components are retained; no raster or generated assets are involved.
- Copy and content: Spec, New spec, and the saved conversation remain unchanged; only the redundant `Free` badge is removed.

## Comparison history

1. The source capture showed excessive rem-scaled height, drifting columns, and a redundant Spec badge.
2. Fixed-pixel layout geometry and a shared icon/label rhythm were implemented.
3. The first rendered pass verified the compact rows but did not include a saved Spec conversation.
4. A local mock Spec conversation was created, the sidebar was resized to match the source width, and the state-matched 200% capture was compared in one combined image.
5. No browser console errors were present. Typecheck and all 620 frontend tests passed.

## Full-view and focused comparison evidence

- Full view: `spec-sidebar-compact-200-matched.png` confirms the sidebar remains usable alongside the document composer at 804 × 516.
- Focused comparison: `spec-sidebar-density-comparison-matched.png` shows the source above and revised sidebar below at equal 434 × 320 pixel dimensions.

## Final result

passed

---

# Design QA — live Spec editing

- Source visual truth: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_3QCEwD/Screenshot 2026-08-13 at 4.49.01 PM.png`
- Browser-rendered implementation: `/Users/stan/Documents/git/clark-desktop/spec-live-editing-implementation-dark.png`
- Combined comparison: `/Users/stan/Documents/git/clark-desktop/spec-live-editing-comparison-dark.png`
- Viewport: 1276 x 956 CSS px at device scale 1.
- Source pixels: 1276 x 957, normalized to 1276 x 956. Implementation pixels: 1276 x 956.
- State: dark theme, active Spec run after the canonical Markdown artifact changed and before terminal settlement.

## Findings and fixes

- [P1] Indefinite working state — the static “Shaping the next section” skeleton did not prove that the document was changing.
  - Fixed by polling the canonical document during active work, rendering saved revisions immediately, and showing real added and removed lines with applied totals.
- [P1] Editable-looking processing state — selection remained available while the agent owned document edits.
  - Fixed with `aria-busy`, a wait cursor, cleared ranges, and `select-none` while active; selection restores after settlement.
- Remaining P0/P1/P2 findings: none.

## Required fidelity surfaces

- Fonts and typography: Clark's DM Sans, Newsreader, JetBrains Mono, and semantic type ramp are preserved.
- Spacing and layout rhythm: the live receipt keeps the prior bottom-center placement and reading-rail width while showing useful content.
- Colors and visual tokens: existing accent, success, danger, surface, and border tokens are used; signs and strike-through keep changes legible without relying on color alone.
- Image quality and asset fidelity: no raster assets are involved; the editing indicator uses the existing icon library.
- Copy and content: “Editing specification,” actual changed lines, and `+ / −` totals replace the indefinite skeleton copy.
- Focused-region comparison was unnecessary because the receipt and surrounding document/composer remain readable in the equal-size full-view comparison.

## Browser checks

- The branded Clark product composition rendered the updated document and live change receipt.
- The active document exposed `aria-busy="true"` and `select-none`.
- Reduced-motion behavior remains owned by the app's shared motion policy.
- The preview used the deterministic mock provider and made no hosted-model call.

## Comparison history

1. The source showed a static skeleton over an otherwise unchanged document.
2. The implementation replaced it with a live revision receipt and updated document content.
3. A dark-theme capture at the source viewport found no actionable P0/P1/P2 mismatch for the requested redesign.

## Final result

passed

---

# Design QA — compact specialist navigation tree

- Source visual truth: `/Users/stan/.codex/generated_images/019ff26c-b1de-7e42-a7e2-e6222d3cfa7d/exec-ec855f16-b1c7-44f4-816a-c9a37be60e6a.png`
- Browser-rendered implementation: `/Users/stan/.codex/visualizations/2026/08/11/019ff26c-b1de-7e42-a7e2-e6222d3cfa7d/specialist-navigation-light-full.png`
- Focused implementation crop: `/Users/stan/.codex/visualizations/2026/08/11/019ff26c-b1de-7e42-a7e2-e6222d3cfa7d/specialist-navigation-light-tree.png`
- Combined comparison: `/Users/stan/.codex/visualizations/2026/08/11/019ff26c-b1de-7e42-a7e2-e6222d3cfa7d/specialist-navigation-light-comparison.png`
- Viewport: 1000 x 700 CSS px at device scale 1.
- Source pixels: 1774 x 887; normalized source region: 756 x 215. Implementation pixels: 1000 x 700; focused region: 192 x 242.
- State: light theme, canonical Clark product composition, RSI active, saved RSI conversation selected, sidebar constrained to 192px.

## Findings and fixes

- [P1] Redundant entitlement chrome — the mock still showed `PRO`, while the user explicitly established that a subscribed account should not repeat its plan inside navigation.
  - Fixed by removing badge rendering, the foundation `specialistBadge` product contract, the Clark product override, and the obsolete catalog badge field. Entitlement remains authoritative in the access projection and gate.
- [P1] Oversized nested rows — the source concept was visually polished but too spacious for the real sidebar.
  - Fixed with one 36px specialist row and one 32px conversation row. The tree stays readable at the real 192px constrained width.
- [P2] Split selected-state language — the old parent and child used separate filled blocks and accent rules.
  - Fixed with one typographic tree: the active specialist uses an accent icon and ink label; the selected conversation uses a quiet tint, matching icon, and shared branch rail.
- Remaining P0/P1/P2 findings: none.

## Required fidelity surfaces

- Fonts and typography: Clark's DM Sans semantic `text-sm` and `text-xs` tokens are retained with compact line height and end truncation; the shared typography-policy test passes.
- Spacing and layout rhythm: 36px parent, 32px child, 14px branch inset, and 8px row padding preserve the mock's hierarchy while honoring the user's density correction.
- Colors and visual tokens: warm paper, graphite ink, `accent`, and `accent-subtle` are all existing Clark tokens; no new palette or gradient was introduced.
- Image quality and asset fidelity: product-provided Lucide-compatible specialist icons and the existing `MessageSquare` icon remain vectors from the icon library; no raster substitute, custom SVG, or placeholder was introduced.
- Copy and content: RSI and the conversation title remain intact. Plan labels and running-status prose are removed from the compact visual surface while their accessible state remains announced.

## Browser checks

- The RSI parent opened and exposed its saved conversation.
- The conversation opened and remained `aria-current="page"`.
- Keyboard focus retained a visible focus outline; the resting selected state matched the quiet-tint target.
- No browser console errors were present. The only observed console messages were the existing reduced-motion development warning.

## Comparison history

1. The chosen mock established the typographic tree but still contained `PRO` and generous concept-canvas spacing.
2. User feedback made badge removal and materially tighter density explicit.
3. The first implementation removed badge chrome and consolidated parent/child styling.
4. The canonical Clark product entry and catalog still exposed obsolete badge metadata; those split-brain inputs were removed and the catalog digest was updated.
5. The final light-theme capture at the real 192px sidebar width showed no actionable P0/P1/P2 mismatch after applying the user's density and entitlement overrides.

## Final result

passed

---

# Design QA — repository-grounded Spec writing

- Source visual truth: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/codex-clipboard-8cd8d9ee-0f99-49cd-99d2-ff263fbe30dc.png`
- Repository/folder mention screenshot: `/Users/stan/.codex/visualizations/2026/08/10/019fed7c-690d-7241-a304-b500474e5b77/spec-repo-folder-mentions-200.png`
- LLM writing-state screenshot: `/Users/stan/.codex/visualizations/2026/08/10/019fed7c-690d-7241-a304-b500474e5b77/spec-writing-skeleton-200.png`
- Combined new-state comparison: `/Users/stan/.codex/visualizations/2026/08/10/019fed7c-690d-7241-a304-b500474e5b77/spec-new-states-comparison.png`
- Source-and-implementation comparison: `/Users/stan/.codex/visualizations/2026/08/10/019fed7c-690d-7241-a304-b500474e5b77/spec-reference-and-repo-mentions.png`
- Implementation viewport: 1280 × 720 px.
- State: light theme, 200% text preference, empty Spec `@` mention menu and active mock-provider Spec writing run.

## Findings and fixes

- [P1] Repository grounding — ordinary `@file` suggestions were previously scoped to the private Spec-document folder, so a repository-looking chip would not have allowed the agent to read product code.
  - Fixed by making `@repo` select and persist the actual local session root before the first turn. `@folder` and file references are constrained to that root and become compact removable context chips.
- [P1] Invisible working state — a document skeleton appended after the full Markdown body was usually below the viewport, making an active run still feel static.
  - Fixed with a calm, always-visible document overlay containing slow staggered text lines. It disappears when the run settles and respects reduced-motion preferences.
- [P1] Enlarged-text shell overflow — the sidebar's content could make the otherwise fixed app shell programmatically scroll when the 200% composer received focus, clipping the header and top navigation.
  - Fixed by containing the sidebar's height and overflow inside its existing conversation scroller. At enlarged text sizes the Spec header retains semantic type while collapsing secondary action labels to icons, keeping the title, filename, and actions on one stable horizontal rhythm.
- [P2] Generated-title pollution — placing the hidden code-context envelope before the user's words allowed the mock title generator to name the Spec from workflow instructions.
  - Fixed by keeping the user's request first and appending the typed context envelope after it.

## Required fidelity surfaces

- Layout: the document remains the primary canvas, the composer remains persistent, and code context stays inside the existing composer rather than adding a new side panel.
- Motion: skeleton pulse is 3.2 seconds with small staggered delays; it is quiet enough for longer model/tool runs and disables under reduced motion.
- Controls: `@repo` and `@folder` use the existing Lucide icon language, focus/hover surfaces, violet accent, and keyboard-selectable autocomplete behavior.
- Accessibility: repository and folder actions have semantic button names; selected references have explicit remove labels; the writing state is a polite live status.
- Real boundary: the browser preview verified autocomplete, chips, busy state, title behavior, and visual layout. The native OS picker itself is not available in browser preview; repository containment and prompt-grounding contracts are covered by unit tests.

## Final result

passed

---

# Design QA — parent-folder references and removable Spec focus

- Source visual truth: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_hLH1te/Screenshot 2026-08-13 at 5.00.04 PM.png`
- Scout source finding: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_xfYmWN/Screenshot 2026-08-13 at 5.00.30 PM.png`
- Browser-rendered implementation: `/tmp/clark-parent-ref-qa.KgEwRY/parent-folder-autocomplete.png`
- Focused implementation crop: `/tmp/clark-parent-ref-qa.KgEwRY/implementation-crop.png`
- Combined comparison: `/tmp/clark-parent-ref-qa.KgEwRY/comparison.png`
- Viewport: 1280 × 720 CSS px, device scale factor 1
- Source pixels: 833 × 222; implementation pixels: 1280 × 720; comparison regions: 833 × 300 each
- State: dark theme, local `clark-desktop` checkout, new-session composer focused, `@../` typed

## Full-view and focused comparison evidence

The full browser capture preserves the existing new-session layout and composer proportions. The
focused combined comparison shows the reported empty `@../` state above and the corrected state
below at equal width: sibling folders appear in the existing autocomplete surface, are explicitly
labeled read-only, and retain a folder-picker fallback.

## Findings and fixes

- [P1] Parent-path trigger ended at the slash, so `@../` produced no suggestions.
  - Fixed by treating slashes inside an `@` token as mention content and adding the parent submenu.
- [P1] A background Spec session leaked its repository chip into Scout.
  - Fixed by making the visible specialist lens authoritative for composer context.
- [P1] Repository focus had no removal affordance or access revocation.
  - Fixed with an accessible × action that revokes the live provider read root before removing the chip.
- Remaining P0/P1/P2 findings: none.

Required fidelity surfaces:

- Fonts and typography: existing sans and monospace composer hierarchy is preserved.
- Spacing and layout rhythm: existing popover row padding, radius, elevation, and composer geometry are reused.
- Colors and visual tokens: existing dark, accent, border, and muted-ink tokens are unchanged.
- Image and asset fidelity: existing Lucide folder and close icons are reused; no raster assets were introduced.
- Copy and content: sibling paths use `../name`, access is labeled `Read-only`, and the fallback is `Choose another folder…`.

## Interaction verification

- Bare `@` exposes `../ Browse folders beside this checkout`.
- Selecting that row opens `../clark`, `../clark-public-evals`, and `Choose another folder…`.
- Selecting `../clark` inserts `@../clark/`.
- Opening Scout no longer renders the background Spec repository chip.
- Spec renders `Remove repository focus clark-desktop`; activating it removes the chip.
- Browser console had no current errors; the only current warning was the expected reduced-motion development warning.

## Native verification

The provider regression proves an admitted external repository is readable but non-writable, then
becomes unreadable again after revocation. The desktop Tauri command surface compiles with both add
and remove operations.

final result: passed

---

# Design QA — guided Spec interview

- Source visual truth: `/Users/stan/.codex/generated_images/019ffd8e-bbb4-71f2-a09a-7ca052b61fc0/exec-19537d0b-f922-4339-aeb1-fc54b4f5c1ba.png`
- Browser-rendered implementation: `/Users/stan/Documents/git/clark-desktop/target/spec-guided-final.png`
- Selected-answer state: `/Users/stan/Documents/git/clark-desktop/target/spec-guided-selected.png`
- Compact implementation: `/Users/stan/Documents/git/clark-desktop/target/spec-guided-compact-final.png`
- Combined comparison: `/Users/stan/Documents/git/clark-desktop/target/spec-guided-final-comparison.png`
- Desktop viewport and source pixels: 1479 × 1064.
- Compact viewport: 900 × 900 CSS px.
- State: dark theme, canonical Clark product composition, new Spec before its first substantive turn, deterministic mock bridge, no hosted-model call.

## Findings and fixes

- [P1] The document-first surface exposed open questions but did not give a nontechnical person an approachable path through them.
  - Fixed with one plain-language, high-information question at a time, three concrete choices, and a free-form answer.
- [P1] A decorative completion count could imply agent readiness without evidence.
  - Fixed by deriving eight decision areas from substantive sections in the canonical Markdown; empty headings and starter prompts do not count.
- [P1] The selected answer did not visibly affect the document until an agent turn completed.
  - Fixed with a lifted in-document shaping cue: selected or narrated words morph into the affected decision area and acceptance coverage before the real Spec workflow reconciles them into the file. The existing full-document revision animation remains the saved-change boundary.
- [P1] A separate interview control risked becoming a local-only wizard.
  - Fixed by routing established conversations through the pinned `spec:spec` skill and the same durable send path. Before a session exists, the first answer moves into and focuses the persistent whole-spec composer so project selection and first-turn setup remain authoritative.
- [P2] Stacking the full-height interview below the document at narrower desktop widths initially left too little reading room and clipped the current question.
  - Fixed with a bounded 56/44 split, a scrollable interview body, two-column choices where space permits, and a compact primary action. Horizontal overflow remains zero at 900px.
- Remaining P0/P1/P2 findings: none.

## Required fidelity surfaces

- Typography: Clark's Newsreader document hierarchy and DM Sans interface text match the reference's editorial document-plus-margin-question composition.
- Layout: the canonical document remains primary, the guided interview occupies the right margin on desktop, and the persistent whole-spec composer remains available.
- Color: the existing violet accent, green readiness tokens, graphite surfaces, and subtle borders are reused without a new palette or gradient.
- Motion: option selection uses the shared rise/layout vocabulary; saved Markdown changes use the existing full-document revision transition; reduced-motion users get the same state changes without spatial movement.
- Image and asset fidelity: no raster illustration was needed; controls use the product's existing icon library.
- Copy and content: questions avoid implementation jargon while generated prompts explicitly preserve unrelated content, the semantic `*_SPEC.md` filename, a decision log, testability, and the no-implementation boundary.

## Browser verification

- Choosing a suggested answer set `aria-pressed`, enabled `Next question`, and projected the answer into the in-document shaping cue.
- A custom answer cleared the selected choice and replaced the cue content after the morph transition settled.
- Before a session exists, `Next question` moved the plain-language decision into the whole-spec composer and focused it without sending or selecting a hosted model.
- Clicking existing document text still opened the exact-selection discussion with `Ask about this` and `Suggest edit` actions.
- Desktop and compact captures had no horizontal overflow or clipped primary controls.
- Browser console errors: none. The only warning was the expected Motion development notice for the system reduced-motion preference.
- Frontend verification: 724 tests passed; typecheck and production build passed.

final result: passed

---

# Design QA — full-document Spec revision animation

- Source visual truth: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/TemporaryItems/NSIRD_screencaptureui_R5cv9s/Screenshot 2026-08-13 at 5.35.06 PM.png`
- Active implementation: `/Users/stan/Documents/git/clark-desktop/spec-document-diff-active.png`
- Removal-collapse state: `/Users/stan/Documents/git/clark-desktop/spec-document-diff-settling.png`
- Stabilized document: `/Users/stan/Documents/git/clark-desktop/spec-document-stable.png`
- Source-and-implementation comparison: `/Users/stan/Documents/git/clark-desktop/spec-document-diff-comparison.png`
- Transition sequence: `/Users/stan/Documents/git/clark-desktop/spec-document-transition.gif`
- Source pixels: 1020 × 855; implementation viewport: 1280 × 720 CSS px.
- State: dark theme, real Clark product entry, deterministic mock Spec run, system reduced-motion preference enabled.

## Findings and fixes

- [P1] The git-style receipt floated above the document and obscured the content it was meant to explain.
  - Fixed by removing the overlay entirely. The document column becomes the revision surface, so each deletion and addition appears at its actual reading position.
- [P1] Large replacements initially grouped all removed lines before all added lines, producing a red block followed by a green block.
  - Fixed by pairing replacement rows around the longest-common-subsequence anchors. Whole-document rewrites now read as old line followed by new line throughout the page.
- [P1] The document remained selectable while the agent owned the edit.
  - Fixed with `aria-busy`, `select-none`, a wait cursor, selection clearing, and guarded click, double-click, and mouse-selection handlers for the duration of the run.
- [P2] Blank diff rows inherited line-number height and color, stretching the transition vertically.
  - Fixed by preserving compact document spacing while suppressing change chrome for whitespace-only rows.
- [P2] Persistent change chrome made the finished document feel like a code-review tool rather than a readable specification.
  - Fixed with a two-phase transition: removed rows collapse first, additions settle into their final positions, and the canonical Markdown replaces the transient surface after 2.05 seconds. Reduced-motion users get the same understandable states without spatial animation.
- Remaining P0/P1/P2 findings: none.

## Required fidelity surfaces

- Typography: the existing Newsreader heading hierarchy, DM Sans body text, and JetBrains Mono diff gutter are retained.
- Layout: the existing document measure and scroll surface remain authoritative; no floating card or parallel diff panel is introduced.
- Color: additions and removals reuse Clark's semantic success and danger tokens at low-opacity backgrounds, with no gradient or new palette.
- Motion: additions use the shared small-rise preset, removals use the shared fade/expand presets, rows use layout motion, and the stable document uses the shared rise preset.
- Accessibility: the transient document is a polite status, non-selectability is semantic and visual, and the reduced-motion path preserves content and timing cues while removing spatial motion.

## Browser verification

- The active state rendered 19 meaningful additions and 19 meaningful removals inside `[data-qa="spec-document"]`.
- Replacement rows alternated remove/add around unchanged anchors; no floating live-edit component existed.
- While processing, `aria-busy="true"` and computed `user-select: none` were present.
- After stabilization, the diff surface count returned to zero, `aria-busy="false"`, and computed `user-select: text` was restored.
- The canonical document title and headings rendered after stabilization.
- Browser console errors: none. The only warning was the expected Motion development notice for the system reduced-motion preference.

final result: passed
