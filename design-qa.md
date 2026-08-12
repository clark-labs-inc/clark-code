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
