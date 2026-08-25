import { describe, expect, it } from "vitest";

const sourceModules = import.meta.glob("../**/*.{ts,tsx}", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

const productionSources = Object.entries(sourceModules).filter(
  ([path]) => !/\.spec\.(ts|tsx)$/.test(path),
);

/** Surfaces exempt from the raw-palette ban, each with the reason. A fixed
 *  dark instrument surface (ComputerUseLiveCard) intentionally owns its own
 *  white/black alpha steps instead of theme-reactive tokens; when such a
 *  surface is redesigned onto tokens, remove its entry here. */
const RAW_PALETTE_EXEMPT = new Set([
  "../surfaces/work/ComputerUseLiveCard.tsx",
]);

// Surfaces still on arbitrary z-index values awaiting migration to the
// documented stacking ladder (.z-elevated / .z-toast / .z-critical).
const ARBITRARY_Z_EXEMPT = new Set([
  "../surfaces/ComposerPermissionPill.tsx",
]);

describe("GUI token policy", () => {
  it("routes every surface color through the semantic palette", () => {
    const violations = productionSources.flatMap(([path, source]) => {
      if (RAW_PALETTE_EXEMPT.has(path)) return [];
      // Raw Tailwind defaults bypass --color-*: they ignore dark mode and
      // interface contrast. Scrims, media pages/stages, and knobs have named
      // tokens; anything else belongs in the @theme palette.
      const raw = source.match(
        /\b(?:bg|text|border|ring|fill|stroke|from|to)-(?:white|black)(?=[\s/"')]|$)/g,
      ) ?? [];
      const named = source.match(
        /\b(?:bg|text|border|ring)-(?:gray|slate|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-\d{2,3}\b/g,
      ) ?? [];
      return [...raw, ...named].map((className) => `${path}: ${className}`);
    });

    expect(violations).toEqual([]);
  });

  it("keeps elevation in shadow tokens, not one-off arbitrary values", () => {
    const violations = productionSources.flatMap(([path, source]) =>
      (source.match(/\bshadow-\[[^\]]+\]/g) ?? []).map(
        (className) => `${path}: ${className}`,
      ),
    );

    expect(violations).toEqual([]);
  });

  it("keeps stacking on the documented z ladder", () => {
    const violations = productionSources
      .filter(([path]) => !ARBITRARY_Z_EXEMPT.has(path))
      .flatMap(([path, source]) =>
        (source.match(/\bz-\[\d+\]/g) ?? []).map(
          (className) => `${path}: ${className}`,
        ),
      );

    expect(violations).toEqual([]);
  });
});
