// @ts-ignore - testing-only Node API, not visible to the browser build.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { DUR, EASE } from "./motion";

const sourceModules = import.meta.glob("../**/*.{ts,tsx}", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

// All JS/TSX sources, minus the policy spec itself (keeps the two original
// checks scoped to code).
const sources = Object.entries(sourceModules)
  .filter(([path]) => !path.endsWith("motionPolicy.spec.ts"))
  .map(([path, source]) => ({ path, source }));

// Sources that must not contain inline motion literals: everything except the
// vocabulary itself and test files (tests legitimately assert on the values).
const banSources = sources.filter(({ path }) =>
  !/(^|\/)motion\.ts$/.test(path) && !/\.spec\.(ts|tsx)$/.test(path)
);

// The Tailwind Vite plugin owns `.css` and returns an empty string for raw
// glob/static imports, so read the real index.css off disk and token-check it.
// Same effect: drive the "keep the two in sync" comment load-bearing.
// The app's tsconfig is browser-scoped (@types/node isn't installed) while
// vitest runs in Node, which provides the `node:fs` builtin at runtime.
// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore - testing-only Node API, not visible to the browser build.
const cssSource: string = readFileSync(
  new URL("../index.css", import.meta.url),
  "utf8",
);

describe("GUI motion policy", () => {
  it("uses lightweight m components behind the strict LazyMotion boundary", () => {
    const violations = sources.filter(({ source }) =>
      /import\s*\{[^}]*\bmotion\b[^}]*\}\s*from\s*["']motion\/react["']/.test(source)
      || /<\/?motion\./.test(source)
    );

    expect(violations.map(({ path }) => path)).toEqual([]);
    expect(sourceModules["../main.tsx"]).toContain(
      "<LazyMotion features={domMax} strict>",
    );
  });

  it("avoids broad CSS transitions that can animate layout accidentally", () => {
    const violations = sources.filter(({ source }) => /\btransition-all\b/.test(source));
    expect(violations.map(({ path }) => path)).toEqual([]);
  });

  it("bans inline exit hard-cuts (the reported duration: 0 exit regression class)", () => {
    // The enter snap is `transition={{ duration: 0 }}` — legitimate. A vanish at
    // `exit={… duration: 0 …}` is a hard cut and must go through REDUCED_EXIT /
    // accessibleMotion instead.
    const violations = banSources.filter(({ source }) =>
      /exit=\{\s*[^}\n]*?duration:\s*0\b/m.test(source),
    );
    expect(violations.map(({ path }) => path)).toEqual([]);
  });

  it("bans raw inline slides/reveals outside the vocabulary", () => {
    // `{ opacity: 0, x|y: … }` (and the scale-chip variants) must be expressed
    // via RISE* / SLIDE_* / FADE presets, never hand-written in a surface.
    const violations = banSources.filter(({ source }) =>
      /\{\s*opacity:\s*0\s*,\s*(?:scale\s*:\s*[0-9.]+\s*,\s*)?[xy]\s*:/.test(source),
    );
    expect(violations.map(({ path }) => path)).toEqual([]);
  });

  it("bans hand-rolled stagger arithmetic", () => {
    // `delay: … index * 0.0X` must go through staggeredTransition so reduced
    // motion can zero the choreography in one place.
    const violations = banSources.filter(({ source }) =>
      /delay:\s*[^,}{\n]*index\s*\*/.test(source),
    );
    expect(violations.map(({ path }) => path)).toEqual([]);
  });

  it("keeps the JS motion tokens in sync with the CSS tokens in index.css", () => {
    expect(cssSource).toContain("--dur-fast:");

    const cssVar = (name: string): string => {
      const line = cssSource.split("\n").find((l) => l.includes(`--${name}:`));
      expect(line, `index.css should define --${name}`).toBeDefined();
      return line!.split(":")[1].trim().replace(/;.*$/, "").trim();
    };

    expect(Number.parseFloat(cssVar("dur-fast"))).toBe(DUR.fast * 1000);
    expect(Number.parseFloat(cssVar("dur-base"))).toBe(DUR.base * 1000);
    expect(Number.parseFloat(cssVar("dur-slow"))).toBe(DUR.slow * 1000);

    const cssEase = (name: string): number[] => {
      const match = cssVar(name).match(/cubic-bezier\(\s*([^)]+)\)/);
      expect(match, `--${name} should be a cubic-bezier`).toBeTruthy();
      return match![1].split(",").map((s) => Number.parseFloat(s.trim()));
    };
    expect(cssEase("ease-clark")).toEqual([...EASE.out]);
    expect(cssEase("ease-clark-inout")).toEqual([...EASE.inOut]);
  });
});
