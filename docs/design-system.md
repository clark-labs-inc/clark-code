# Clark Code design system

The design system is three layers: **tokens** (the vocabulary, defined once),
**primitives** (shared CSS classes and motion presets built on those tokens),
and **policies** (tests that keep every surface inside the vocabulary). If you
are adding a surface, everything visual it uses should resolve to a token; if
it can't, the token is missing — add one rather than inlining a value.

## Layer 1 — Tokens (`app/src/index.css`, `@theme`)

### Color

Semantic palette (theme-reactive): `bg`, `bg-secondary`, `bg-tertiary`,
`bg-elevated`, `bg-hover`, `bg-sunken`, `composer-context`,
`composer-surface`; ink tiers `ink`, `ink-secondary`, `ink-muted`,
`ink-faint`; interaction `accent`, `accent-hover`, `accent-soft`,
`accent-subtle`, `accent-focus`, `on-accent`; status `success`, `warning`,
`danger`, `info`; chrome `chip`, `border`, `border-subtle`, `border-strong`;
checkout `checkout-branch`, `checkout-worktree`. Every tier has light, dark,
contrast, and colorblind variants defined in the same file.

Fixed surfaces that deliberately do **not** follow light/dark:

| Token | Value | Use |
| --- | --- | --- |
| `--color-scrim` | black 40% | modal overlays |
| `--color-scrim-strong` | black 55% | command palette overlay |
| `--color-media-page` | white | literal paper content (PDF pages, document previews) |
| `--color-media-stage` | black | video stages |
| `--color-knob` | white | switch knobs on tinted tracks |

Raw Tailwind colors (`bg-white/10`, `text-red-300`, …) bypass dark mode and
interface contrast and are banned by policy.

### Type

Ramp: `text-xs` (12) · `text-sm` (13) · `text-base` (15) · `text-lg` (17) ·
`text-xl` (20) · `text-2xl` (24) · `text-3xl` (30), all scaled by the
user-facing text-size preference via `--text-size-scale`. Arbitrary sizes
(`text-[14px]`, `text-[0.85em]`, …) are banned; relative-to-context sizing
lives in shared CSS classes instead.

Fonts: DM Sans (UI), Newsreader (display emphasis), JetBrains Mono (literal
data). Inline code gets one shared chip from the global
`code:not(pre code)` rule — radius, chip background, padding, family, and
relative size are system-owned; callers set only color.

### Radius

`xs` 5 (inline chips) · `sm`/`md` 6 (compact controls) · `lg`/`xl` 8 ·
`2xl` 12 (floating overlays). No arbitrary radii.

### Shadow

`soft` (none) · `lifted` (popovers/menus) · `deep` (sunken artifact previews)
· `float` (floating instrument panels) · `button` (secondary CTA lift) ·
`glow-accent` (primary CTA glow; follows `--color-accent`). Arbitrary
`shadow-[…]` is banned.

### Motion

CSS owns `--dur-fast/base/slow` (120/200/300 ms), `--ease-agent` (house
ease-out), `--ease-agent-inout`. `app/src/lib/motion.ts` mirrors them in
seconds for motion/react and exposes the presets (`FADE`, `RISE`,
`RISE_SMALL`, `EXPAND`, `OVERLAY`, `DIALOG`, `SLIDE_LEFT/RIGHT`,
`accessibleMotion`, `staggeredTransition`, `indeterminateTransition`).
A policy test asserts the two files never drift.

### Stacking ladder

```
0–10   inline content
20–30  sticky chrome
40     dropdowns
50     modals + popovers        z-50
70     elevated chrome above open popovers   .z-elevated
90     toasts                                .z-toast
100    critical, tops everything             .z-critical
```

New layers join this ladder. Arbitrary `z-[N]` is banned (one migration
exemption remains in `tokenPolicy.spec.ts`).

## Layer 2 — Policies

Static-analysis tests over production sources:

| Spec | Enforces |
| --- | --- |
| `lib/motionPolicy.spec.ts` | no inline motion literals, no `transition-all`, JS↔CSS token sync, reduced-motion contract |
| `lib/typographyPolicy.spec.ts` | no arbitrary `text-[…]` sizes |
| `lib/tokenPolicy.spec.ts` | semantic palette only (with explicit exemptions), shadow tokens only, documented z ladder |
| `lib/layoutPolicy.spec.ts` | structural layout contracts |

Each exemption carries its reason next to the entry. When you migrate an
exempt surface onto tokens, delete its exemption — the lists should trend to
empty.

## Adding a visual value

1. Does a token exist? Use it.
2. Otherwise ask whether it's a *system* value (add to `@theme`, with a
   comment explaining what role it plays) or a one-off (usually means the
   design is wrong — reuse an existing tier).
3. Run `pnpm test` — the policies fail with file paths if you drifted.
