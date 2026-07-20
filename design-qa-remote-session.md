# Design QA: Remote New Session

- Source visual truth: `/Users/stan/.codex/generated_images/019f8079-e143-7d60-96e0-0e070060f29e/exec-6c4eab1a-83df-4cc7-824a-762b977309bf.png`
- Implementation screenshot: `/Users/stan/.codex/visualizations/2026/07/20/019f8079-e143-7d60-96e0-0e070060f29e/remote-session-implementation.jpg`
- Full-view comparison: `/Users/stan/.codex/visualizations/2026/07/20/019f8079-e143-7d60-96e0-0e070060f29e/remote-session-comparison.jpg`
- Focused sidebar comparison: `/Users/stan/.codex/visualizations/2026/07/20/019f8079-e143-7d60-96e0-0e070060f29e/remote-session-sidebar-comparison.jpg`
- Viewport: 1440 x 1024, dark mode
- State: blank session composer targeted to `ubuntu@cpu` at `/home/ubuntu/clark`, with the remote project row hovered

## Findings

No actionable P0, P1, or P2 mismatches remain.

- Fonts and typography: the implementation keeps Clark Desktop's existing DM Sans and Newsreader hierarchy. The sidebar label, host name, and path retain the production type scale instead of copying rasterized ImageGen lettering.
- Spacing and layout rhythm: the square-pen action appears immediately before the three-dot menu, matching the selected direction. The production 272 px sidebar and existing start-card/composer placement are intentionally preserved; the generated mock's wider sidebar and centered composer were conceptual, not changes required for remote-session parity.
- Colors and visual tokens: hover, focus, icon, border, and context-chip colors use the existing Clark tokens. The remote action receives the same restrained purple focus treatment shown in the source.
- Image quality and asset fidelity: there are no raster assets in this feature. Existing Lucide project, server, square-pen, and menu icons are reused at the product's native sizes.
- Copy and content: the action exposes the exact accessible label `New session on ubuntu@cpu`. The destination state visibly retains `ubuntu@cpu` and `/home/ubuntu/clark` in the existing context chips. The mock's extra explanatory sentence is intentionally omitted because the production chips already communicate execution location without duplicating copy.

## Interaction Verification

- Hovering the remote project row reveals exactly one `New session on ubuntu@cpu` action before the project menu.
- Clicking it returns to the blank composer while preserving the `ubuntu@cpu` host and `/home/ubuntu/clark` remote root.
- The tested page produced no browser console errors. A reduced-motion environment warning was unrelated to this change.
- A removed or renamed SSH preset opens Remote hosts and shows a recovery notice instead of silently starting against the wrong machine.

## Comparison History

- Initial full-view and focused-region comparison found no P0/P1/P2 mismatch in the requested interaction, hierarchy, host/path visibility, icons, or tokens.
- No visual fixes were required after the comparison. The implementation evidence above is the passing capture.

## Follow-up Polish

- P3: the operating system's native `title` tooltip was not visible in the browser capture, although the exact `title` and `aria-label` contracts are covered by the component test.

## Implementation Checklist

- [x] Remote project rows expose the one-click new-session action.
- [x] The action resolves the saved SSH destination independently of a renamed display label.
- [x] Matching prefers the prior remote root when the same SSH destination has multiple presets.
- [x] Local and remote project shortcuts explicitly select the correct execution mode.
- [x] Unit tests, TypeScript checks, production build, browser interaction, and visual comparison pass.

final result: passed
