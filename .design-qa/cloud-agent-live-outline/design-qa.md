# Clark Cloud Agent live outline design QA

- Source visual truth: `/var/folders/sx/5vx_xn5j0c31t3jw27sjtnqc0000gn/T/codex-clipboard-e369848a-74ad-4f2d-9d4e-8c4258fbcfca.png`
- Source viewport: `2170 x 725` pixels
- Implementation screenshot: unavailable
- Combined comparison: unavailable
- Target state: active Clark Cloud Agent response with one current phase expanded and two parallel research agents

## Source review

The reference uses the existing Clark Cloud Agent card shell, Clark mark,
purple top rule, dark theme, and compact typography. The live header carries
the latest public activity, an uppercase `LIVE` indicator, and an upward
collapse chevron. The body presents a quiet one-line research query, flat
phase rows with explicit right-aligned statuses, only the current phase's
nested steps, and flat aligned rows for parallel agents.

## Implementation evidence

The source implementation preserves the existing card shell and renders the
selected hierarchy from typed public progress data. Focused static-render tests
cover latest header activity, `aria-live`, automatic current-phase expansion,
collapsed pending-phase details, aligned agent content, completed-outline
retention, the initial starting state, and the reduced-motion path. TypeScript
checking and the focused Vitest suite pass.

## Render blocker

The exact locked frontend dependency set currently fails the repository's
minimum-release-age policy for 13 newly published `lightningcss` and
`enhanced-resolve` entries. The policy was not relaxed or bypassed. Because
Vite/Tailwind cannot be launched through the approved dependency gate, a
post-implementation browser screenshot and same-viewport combined comparison
cannot be produced in this run.

## Findings

- [P1] Rendered fidelity remains unverified until the dependency release-age
  gate clears or the dependency upgrade is changed to an accepted lockfile.
- No source-level hierarchy, typed-contract, accessibility, or reduced-motion
  defects remain in the focused checks.

## Required follow-up

1. Run the production frontend build after the dependency gate accepts the
   lockfile.
2. Render the live state at the reference viewport.
3. Place the reference and implementation captures into one comparison image.
4. Correct any material spacing, border, type, or alignment differences and
   mark this report passed only after the combined comparison is clean.

final result: blocked
