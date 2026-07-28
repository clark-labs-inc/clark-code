# Clark Temporal Atlas design QA

## Visual truth

- Source: `/Users/stan/.codex/generated_images/019f9816-d089-79d1-b59a-03ea3c64c3ea/call_twL687WBjDyugtGXOLhWzyG2.png`
- Source pixels: 1487 x 1058
- Normalized source: 1440 x 1024 at device scale factor 1
- Implementation: `qa-temporal-atlas-final.png`
- Implementation pixels: 1440 x 1024
- CSS viewport: 1440 x 1024 at device scale factor 1
- State: Netflix simulated enterprise twin, Day 3 at 14:32, simulation overlay selected, discovery playback paused for deterministic comparison
- Full-view comparison: `qa-temporal-atlas-comparison-final.png`
- Focused comparison: not required because the complete 2880 x 1024 comparison board keeps the graph nodes, controls, timeline, inspector, and typography readable.

## Findings

No actionable P0, P1, or P2 differences remain.

- Fonts and typography: Newsreader and Inter preserve the source's editorial/product hierarchy. Labels and node metadata remain readable at the target viewport.
- Spacing and layout rhythm: the top bar, enterprise command row, live ribbon, map/inspector split, and timeline match the source's major-region proportions. The entire primary experience fits without vertical or horizontal overflow.
- Colors and visual tokens: warm off-white surfaces, restrained violet coverage, gray dashed boundaries, green confidence, and fine neutral dividers match the selected direction.
- Image and asset fidelity: the screen uses the existing font stack and Tabler icon library. No placeholder imagery, handcrafted SVG, gradient, emoji, or rasterized UI is present.
- Copy and content: the implementation preserves the simulated Netflix framing, Day 3 timeline state, current playback failure scenario, 63% coverage, explicit uncovered boundary, and next-discovery prompt.
- Interaction fidelity: playback, time-step selection, observed/inferred/simulation modes, Day 0 replay, and simulation execution are all functional.

## Comparison history

1. Initial browser capture found two P2 issues: horizontal card-shaped graph nodes were less legible than the source's icon-and-label nodes, and overlapping region fills made the coverage overlay visually muddy.
   - Fix: converted graph entities to vertically stacked icon-and-label nodes and moved violet coverage to one coherent system-level overlay with transparent region boundaries.
   - Evidence: `qa-temporal-atlas-revised.png`.
2. Revised capture found a P2 viewport issue: the final portion of the timeline required scrolling at 1440 x 1024.
   - Fix: recalculated the map height against the viewport so all persistent controls fit on one screen.
   - Evidence: `qa-temporal-atlas-final-fit.png`; browser measurement reported `scrollHeight === innerHeight === 1024`.
3. The fitted capture found a P2 semantic mismatch: Studio Operations and Billing were described as mapped-but-not-covered but were not yet visible at Day 3.
   - Fix: brought those discovered entities and their hypothesis boundaries into the Day 3 graph while leaving them outside the violet simulation contract.
   - Post-fix evidence: `qa-temporal-atlas-final.png` and `qa-temporal-atlas-comparison-final.png`.

## Primary interactions tested

- Day 0 selection: 2 seed entities and 0% simulation coverage.
- Week 1 selection: 18 discovered entities before the Day 3 mapped-boundary refinement and 72% coverage.
- Observed mode: inferred entities removed and simulation coverage reset to 0%.
- Simulation overlay: coverage restored and covered/uncovered regions differentiated.
- Run simulation: state transitioned from Ready to Running to Complete.
- Discovery playback: play, pause, timeline-step selection, and Replay from Day 0.
- Runtime console errors: none.

## Follow-up polish

- P3: the implementation uses orthogonal graph routing for stable interactive behavior while the source uses more organic curved paths.
- P3: the deterministic comparison begins in Ready state instead of the source's Running badge so the primary action remains immediately testable.

## Final result

passed
