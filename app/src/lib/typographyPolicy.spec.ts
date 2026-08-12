import { describe, expect, it } from "vitest";

const sourceModules = import.meta.glob("../**/*.{ts,tsx}", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

const productionSources = Object.entries(sourceModules).filter(
  ([path]) => !/\.spec\.(ts|tsx)$/.test(path),
);

describe("GUI typography policy", () => {
  it("keeps absolute font sizes on the shared semantic type ramp", () => {
    const absoluteArbitrarySize = /\btext-\[[0-9.]+(?:px|rem)\]/g;
    const violations = productionSources.flatMap(([path, source]) => {
      const matches = source.match(absoluteArbitrarySize) ?? [];
      return matches.map((className) => `${path}: ${className}`);
    });

    expect(violations).toEqual([]);
  });
});
