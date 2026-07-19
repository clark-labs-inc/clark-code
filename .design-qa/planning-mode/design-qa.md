# Planning mode design QA

- Source visual truth: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/codex-clipboard-5f3d64dd-c635-4781-9b14-e04595cd2c74.png`
- Implementation screenshot: `/Users/stan/Documents/git/clark-desktop/.design-qa/planning-mode/implementation-final.png`
- Full-view comparison: `/Users/stan/Documents/git/clark-desktop/.design-qa/planning-mode/comparison-final.png`
- Focused comparison: `/Users/stan/Documents/git/clark-desktop/.design-qa/planning-mode/focused-comparison-final.png`
- Viewport: `779 x 1267`
- State: light theme, proposed plan with six numbered steps, request-changes editor open and focused

## Full-view comparison evidence

The reference and browser-rendered implementation were normalized to the same
viewport and placed in one image. Both use a flat editorial plan document, a
small step count, a thin violet header rule, readable numbered markdown, one
decision divider, and a compact approval dock. Dynamic plan copy differs in the
preview fixture, so vertical placement caused by line wrapping is not treated
as a component mismatch.

The source includes the persistent conversation composer below the plan. The
focused preview intentionally renders only the changed permission-gate surface;
the existing composer and conversation width remain unchanged in the app.

## Focused comparison evidence

The dock crops confirm the same hierarchy and action order: `Ready to proceed?`
sits outside the dock border; the editor has a label and an explicit X; keyboard
help and `Send feedback` share one row; and `Run the plan`, `Review each step`,
and `Request changes` remain visible below.

The source mock used a stacked violet textarea border and halo. The final
implementation intentionally differs at the user's direction: the focused
textarea has one 1px violet border, `outline: none`, and no box shadow.

## Required fidelity surfaces

- Fonts and typography: existing DM Sans and JetBrains Mono assets are reused.
  Plan prose uses the established compact 13px editorial rhythm; headings,
  numbered steps, strong text, and code chips preserve the reference hierarchy.
- Spacing and layout: the plan itself has no outer card. One divider separates
  content from the decision area, and only the actionable dock is bordered.
  Buttons stay on one row at the target width and wrap without horizontal
  overflow at `375px`.
- Colors and tokens: all surfaces use Clark's paper, ink, border, and violet
  semantic tokens. Violet is limited to the header rule, active field border,
  primary action, and request-changes affordance.
- Image and icon fidelity: the reference contains no raster content. Controls
  use the project's installed Lucide icon family; no custom SVG, CSS art, or
  placeholder imagery was added.
- Copy and content: static decision copy is concise and explicit about the two
  execution modes. The plan body remains provider-authored markdown.

## Interaction and accessibility evidence

- `Request changes` opens the editor and moves focus to the labeled textarea.
- Escape closes only the editor: the plan alert dialog and approval actions
  remain mounted.
- The X control has the accessible name `Close feedback without changing the
  plan` and produces the same non-destructive close behavior.
- The textarea's focused computed style is a single 1px accent border with no
  visible outline or box shadow.
- At `375 x 812`, `scrollWidth` equaled `clientWidth` (`375px`), so the surface
  introduced no horizontal overflow.
- The browser console reported no errors.

## Findings

No actionable P0, P1, or P2 differences remain. The single focus border is an
intentional improvement over the selected mock based on the user's final visual
direction. The preview fixture's dynamic plan text and omission of unchanged
surrounding composer chrome are accepted scope differences.

## Comparison history

- Pass 1: approval actions wrapped at the target width, increasing dock height.
  Fixed button bases and tightened gaps so all three actions share one row.
- Pass 2: the plan typography was too loose and markdown horizontal rules used
  the browser default dark stroke. Tightened the editorial rhythm and mapped
  rules to Clark's subtle border token.
- Pass 3: removed the stacked textarea border, Tailwind focus ring, and global
  focus outline in favor of one accessible accent border. Focused comparison
  also exposed that `Ready to proceed?` was inside the dock border.
- Pass 4: moved the decision prompt outside the dock border to match the source
  hierarchy and reduce chrome.
- Pass 5: reduced the editor to two rows, matching the selected density. The
  final browser capture found no remaining P0/P1/P2 issue.

## Follow-up polish

None required for this scope.

final result: passed
