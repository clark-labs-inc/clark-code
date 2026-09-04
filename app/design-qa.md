# Sidebar option 2 design QA

## Evidence

- Source visual truth: `/Users/stan/.codex/generated_images/01a069cd-013a-7c42-8b97-231b84876a8a/exec-e2c110cf-942d-4056-9843-dbcb4649f11d.png`
- Browser-rendered implementation screenshot: `/Users/stan/.codex/visualizations/2026/09/04/01a069cd-013a-7c42-8b97-231b84876a8a/sidebar-option-2-implementation.png`
- Full browser screenshot: `/Users/stan/.codex/visualizations/2026/09/04/01a069cd-013a-7c42-8b97-231b84876a8a/sidebar-option-2-implementation-full.png`
- Side-by-side comparison: `/Users/stan/.codex/visualizations/2026/09/04/01a069cd-013a-7c42-8b97-231b84876a8a/sidebar-option-2-comparison.png`
- Browser URL: `http://localhost:1421/?sidebar-fixture`
- Browser viewport: 1280 x 720 CSS pixels at device scale factor 1.
- Sidebar viewport: 385 x 720 CSS pixels; implementation capture normalized to 386 x 720 pixels.
- Source pixels: 918 x 1713. The source was normalized to 386 x 720 so both sides use the same crop, aspect ratio, and density.
- State: dark theme, Specialist Lenses collapsed, Quick chats expanded, first Quick chat selected, project rows collapsed, Archived collapsed.

## Full-view comparison evidence

The selected direction and implementation are shown together in `sidebar-option-2-comparison.png`, with the source on the left and the implementation on the right. The comparison confirms the same recency-first hierarchy, compact Projects heading and primary action, persistent search, expanded Quick chats group, collapsed project rows with counts, vertical child guide, neutral leaf markers, and violet selected-row treatment.

No additional focused crop was needed: at the normalized 386 x 720 size, section labels, project counts, chevrons, child-row markers, search styling, selected-state edge, and archive affordance are all legible in the full comparison.

## Required fidelity surfaces

- Fonts and typography: passed. The implementation uses the existing variable DM Sans system, with 13–15px tokenized sidebar sizes, restrained uppercase labels, consistent weights, and single-line truncation. It is slightly denser than the generated mock, intentionally matching Clark Code's existing desktop type scale.
- Spacing and layout rhythm: passed. Header, actions, search, group headers, child rows, and bottom controls remain on a consistent compact rhythm. The selected row and expanded child guide align cleanly without changing row width.
- Colors and visual tokens: passed. All surfaces use the existing warm graphite semantic tokens; selection uses `accent` and `accent-subtle`, and the search field uses the existing sunken/background and border tokens. No new raw palette values were introduced.
- Image quality and asset fidelity: passed. This navigation contains no raster imagery. Existing Lucide icons are used consistently with the product; no custom SVG, CSS art, emoji, or placeholder assets were added.
- Copy and content: passed. The fixture reproduces the source's Quick chats and selected audio conversation while the product keeps its existing Clark Code, New session, project, archive, and account labels.

## Findings

No actionable P0, P1, or P2 differences remain.

- [P3] Existing product chrome adds a compact Clark Code header and Quick Chat action above the mock's crop. This is an intentional product constraint and preserves global search, collapse, project creation, artifacts, and account behavior.
- [P3] The generated mock includes a separate filter glyph and `RECENT` caption. The implementation keeps one working fuzzy-search field and avoids adding an inert filter control or redundant heading.
- [P3] The QA fixture shows fewer project names than the concept image, but preserves the same collapsed-row density, count alignment, and disclosure interaction.

## Comparison history

1. Initial comparison found a P2 state mismatch: the deterministic sidebar fixture did not contain Quick chats, so the implementation could not reproduce the mock's leading expanded group. The fixture was updated with two realistic Quick chats.
2. Opening a fixture Quick chat initially exposed a mock-only workspace as a separate project. The browser mock workspace path was aligned with the production Quick Chat identity contract, and a regression test was added.
3. The final dark-theme comparison uses the selected first Quick chat state. Quick Chat grouping remains stable after opening, and no P0/P1/P2 mismatch remains.

## Interaction and runtime checks

- Expanded and collapsed a project group.
- Searched for a project, verified matching groups expanded, and cleared the search with Escape.
- Opened the first Quick chat and verified the active selected-row styling.
- Checked browser console output: no errors. The only warning reports that the host has Reduced Motion enabled, which is expected and the UI honors.

## Follow-up polish

- Consider a project-level running marker only if it can summarize real child activity without introducing ambiguous status colors.

final result: passed
