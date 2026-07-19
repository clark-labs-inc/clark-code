# Composer context bar design QA

- Source visual truth: `/Users/stan/Desktop/Screenshot 2026-07-18 at 8.34.08 PM.png`
- Full implementation screenshot: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-refined-full.png`
- Focused implementation screenshot: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-refined.png`
- Combined comparison: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-refined-comparison.png`
- Final color-coded implementation screenshot: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-clean-color.png`
- Final focused screenshot: `/Users/stan/Documents/git/clark-desktop/.design-qa/02-icon-color-audit.png`
- Final source/implementation comparison: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-clean-comparison.png`
- Before/after audit comparison: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-color-before-after.png`
- Colored-label implementation screenshot: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-colored-label.png`
- Colored-label focused comparison: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-colored-label-comparison.png`
- Light-mode implementation screenshot: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-light-final.png`
- Light-mode focused screenshot: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-light-focus.png`
- Dark/light theme comparison: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-theme-comparison.png`
- Light project-picker screenshot: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-light-project-open.png`
- Light environment-picker screenshot: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-light-environment-open.png`
- Centered new-session screenshot: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-centered-light.png`
- Centered active-run screenshot: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-centered-running-light.png`
- Centered completed-session screenshot: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-centered-complete-light.png`
- Centered dark-mode screenshot: `/Users/stan/Documents/git/clark-desktop/.design-qa/composer-context-bar-centered-dark.png`
- Viewport: 1280 × 720
- State: authenticated browser-preview workspace, light and dark themes, local
  `clark-desktop` checkout on `main`, new-session, active-run, and completed-session composers

## Full-view comparison evidence

The implementation keeps Clark Desktop's existing 672px composer width while
matching the source proportions after normalization: a 648px bar, 12px side
insets, 36px exposed header, 90px composer, and 20px radii. The bar stays behind
the composer and does not move persistent controls.

## Focused region comparison evidence

The combined comparison verifies the readable component details at the same
dark interaction state. Project/worktree, execution location, and branch appear
in the same left-to-right order. Their three 14px icons share an identical
vertical coordinate, and their normalized horizontal starts are within 1.5px
of the source.

## Required fidelity surfaces

- Fonts and typography: existing DM Sans UI type is preserved; context labels
  use the product's 12px medium-weight text scale with single-line truncation.
- Spacing and layout rhythm: the bar is inset 12px from the composer, exposes
  36px above it, uses the same 20px corner geometry as the composer, and centers
  the complete metadata group within the 648px bar. Browser measurements put
  its center within 0.01px of the bar center in every tested state.
- Colors and visual tokens: dedicated semantic tokens reproduce the source's
  exact dark surfaces (`#1f1f1f` context and `#2d2d2d` composer) while retaining
  the existing warm light-theme surfaces. The requested state enhancement uses
  Clark's semantic success tint for the regular branch glyph and label, and its
  existing violet accent tint for the linked-worktree fork glyph and label. No
  background, border, ring, padding, or radius is added. The colorblind theme
  remaps success independently from the violet worktree accent. Light mode uses
  checkout-specific `#4f664a` branch and `#5b49cb` worktree colors against the
  `#e6e2da` context surface, producing 4.88:1 and 4.99:1 contrast respectively.
- Image quality and asset fidelity: no raster assets are required; all three
  symbols use the repository's existing Lucide icon dependency.
- Copy and content: the browser mock shows `clark-desktop · Local · main`; the
  native path returns the actual worktree root, local/remote location, branch,
  and detached-HEAD state.

## Findings

No actionable P0, P1, or P2 visual differences remain. The component width is
the existing Clark Desktop constraint; dimensions inside that width are scaled
to the reference. The icon-only state color is an intentional semantic
enhancement requested after the screenshot-matching pass, while preserving the
reference's borderless metadata treatment.

## Interaction and console checks

- New-session project and environment controls remain interactive inside the
  bar; the project popover opens above the composer without clipping.
- Live-session state was tested and correctly becomes read-only while retaining
  the same checkout fields. During a deliberately long mock run, the banner
  retained `clark-desktop · Local · main`, exposed zero buttons, and reported
  its read-only state; the same result held after completion.
- The branch probe refreshes when the checkout changes and after run status
  transitions. Same-checkout refreshes preserve the last resolved identity, so
  the branch/worktree label does not blink out during run-state changes.
- Browser console: no errors. The only warning was the existing Motion reduced-
  motion environment notice.
- Light-mode theme switching, project picker, and environment picker were
  exercised in the live browser. Both popovers remain above the composer without
  clipping, and closing them returns the context row to its original geometry.
- The final dark/light comparison confirms identical 648px-by-48px context-bar
  and 672px composer geometry across themes.

## Comparison history

1. Initial pass found a P2 layering issue: the project popover was clipped by
   the bar and painted behind the composer.
2. Removed the bar's clipping and stacking context. Post-fix evidence in the
   full implementation screenshot showed the popover above the composer.
3. User review identified remaining P1/P2 polish drift: the composer was darker
   than the header, component height was compressed, radii differed, and branch
   content sat above the other two baselines.
4. Added exact dark surface tokens, normalized the 36px/90px vertical geometry,
   aligned all icons to the same y-coordinate, and tuned horizontal starts to
   within 1.5px of the scaled source. The refined comparison is the post-fix
   evidence.
5. Added a low-opacity semantic chip without changing the checkout indicator's
   footprint: regular branches use green plus the branch glyph, while linked
   worktrees use violet plus the fork glyph. The final dark-theme capture shows
   the regular-branch state at 28px high with the original bar alignment intact.
6. User review identified that the chip still looked like a focused control and
   introduced an inconsistent border/radius language. Removed the background,
   ring, padding, and rounded container; retained color only on the branch/fork
   glyph. The before/after audit shows the final metadata row matching the
   source's quiet, borderless rhythm. The item remains 28px high.
7. User requested stronger differentiation without restoring chrome. Extended
   the existing icon color to the branch/worktree label: green for a normal
   branch and violet for a linked worktree. The focused comparison confirms the
   row remains borderless and its geometry is unchanged.
8. Light-mode testing found the original semantic colors measured only 4.23:1
   for branch and 4.14:1 for worktree on the warm context surface. Added scoped
   light checkout tokens at 4.88:1 and 4.99:1, retained the existing dark hues,
   and re-ran the theme switch and both context popovers. Post-fix browser
   screenshots show no layout or layering regression.
9. Centered the metadata group and exercised a long mock run. The active-run
   and completed-session captures show the same project, location, and branch
   as read-only information with no nested controls. Light and dark measurements
   both landed within 0.01px of center, and run-state refreshes no longer clear
   the previously resolved checkout label while the refreshed value loads.

## Follow-up polish

No P3 follow-up is required.

final result: passed
