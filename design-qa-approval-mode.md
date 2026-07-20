# Approval mode popover design QA

- Source visual truth: `/Users/stan/Desktop/Screenshot 2026-07-20 at 10.04.10 AM.png`
- Implementation screenshot: unavailable
- Viewport: compact Clark Desktop window, approximately 401 x 365 CSS pixels at 2x capture scale
- State: local project selected; approval mode menu open; `Approve for me` selected

## Full-view comparison evidence

The source screenshot was opened at original resolution. It shows the checkout path chip painting above the popover title, oversized selected-state panels, and descriptions wrapping across too many lines. A post-fix implementation capture could not be produced because the current `app/pnpm-lock.yaml` dependency upgrade fails the repository's minimum-release-age policy for 13 newly published packages. That policy was not bypassed.

## Focused region comparison evidence

The approval-mode popover was inspected as the focused region. Source-level fixes remove the competing context-bar stacking layer, reduce menu density and corner radius, shorten descriptions, constrain the menu to the viewport, and add roving keyboard focus. Visual comparison remains blocked without a rendered post-fix capture.

## Findings

- [P1] Checkout context paints through the popover header in the source capture.
  - Fix made: removed the context bar's parent stacking layer so composer popovers can render above its chips while context popovers retain their own z-index.
- [P2] The menu reads as three large cards instead of a compact radio menu.
  - Fix made: reduced row padding, type size, icon size, radius, and header emphasis.
- [P2] Explanatory copy wraps excessively in the compact window.
  - Fix made: shortened all three policy descriptions without changing their meaning.
- [P2] Arrow-key navigation is absent from the radio-menu interaction.
  - Fix made: focus the selected option on open and support Arrow Up, Arrow Down, Home, and End.

## Comparison history

1. Initial source review found the P1 stacking collision and three P2 usability/accessibility issues.
2. Source fixes were applied and passed TypeScript checking plus an approval-policy smoke check.
3. Post-fix visual comparison is blocked because launching the current dependency set would require bypassing the repository's supply-chain policy.

## Implementation checklist

- [x] Remove the conflicting context-bar stacking layer.
- [x] Tighten menu hierarchy and density.
- [x] Improve responsive width behavior.
- [x] Add keyboard navigation.
- [x] Pass source/type verification.
- [ ] Capture and compare the post-fix menu after the dependency release-age gate clears or the dependency upgrade is changed.

final result: blocked
