import { describe, expect, it } from "vitest";
import {
  fuzzyScore,
  fuzzyFilter,
  fuzzyFilterFiles,
  fuzzyFilterProjectPaths,
} from "./fuzzy";

describe("fuzzyScore", () => {
  it("matches a subsequence and reports positions", () => {
    const m = fuzzyScore("ac", "abc");
    expect(m).not.toBeNull();
    expect(m!.positions).toEqual([0, 2]);
  });

  it("returns null when the query is not a subsequence", () => {
    expect(fuzzyScore("xyz", "abc")).toBeNull();
  });

  it("an empty query matches anything with no positions", () => {
    expect(fuzzyScore("", "anything")).toEqual({ score: 0, positions: [] });
  });

  it("scores consecutive runs above scattered matches", () => {
    const run = fuzzyScore("abc", "abcxyz")!.score;
    const scattered = fuzzyScore("abc", "axbxc")!.score;
    expect(run).toBeGreaterThan(scattered);
  });

  it("rewards matches at path-segment boundaries", () => {
    const boundary = fuzzyScore("main", "src/main.rs")!.score;
    const mid = fuzzyScore("main", "xxmainxx")!.score;
    expect(boundary).toBeGreaterThan(mid);
  });
});

describe("fuzzyFilter", () => {
  const files = ["src/main.rs", "src/lib.rs", "README.md", "src/store/sessionStore.ts"];

  it("ranks the closest path first", () => {
    const got = fuzzyFilter(files, "sess", (f) => f);
    expect(got[0].item).toBe("src/store/sessionStore.ts");
  });

  it("passes everything through (capped) for an empty query", () => {
    const got = fuzzyFilter(files, "  ", (f) => f, 2);
    expect(got).toHaveLength(2);
    expect(got[0].item).toBe(files[0]);
  });

  it("drops non-matches", () => {
    const got = fuzzyFilter(files, "zzz", (f) => f);
    expect(got).toHaveLength(0);
  });
});

describe("fuzzyFilterFiles", () => {
  const files = ["src/store/sessionStore.ts", "src/surfaces/Composer.tsx", "README.md"];

  it("prefers a basename match over a scattered path match", () => {
    // 'ses' is consecutive in sessionStore's basename; only scattered in the
    // Composer path — the basename hit should win.
    expect(fuzzyFilterFiles(files, "ses")[0]).toBe("src/store/sessionStore.ts");
  });

  it("still matches on the directory portion of a path", () => {
    expect(fuzzyFilterFiles(files, "surf")).toContain("src/surfaces/Composer.tsx");
  });

  it("returns the head of the list (capped) for an empty query", () => {
    expect(fuzzyFilterFiles(files, "", 2)).toEqual(files.slice(0, 2));
  });
});

describe("fuzzyFilterProjectPaths", () => {
  const files = [
    "nucleus-canvas/TODO.md",
    "nucleus-canvas/src/App.tsx",
    "notes/nucleus-canvas-review.md",
  ];

  it("puts matching directories before files within the shared limit", () => {
    const matches = fuzzyFilterProjectPaths(files, "nucleus-canvas", 8);

    expect(matches[0]).toEqual({ kind: "directory", path: "nucleus-canvas" });
    expect(matches.findIndex((match) => match.kind === "file"))
      .toBeGreaterThan(matches.findLastIndex((match) => match.kind === "directory"));
  });

  it("derives each nested directory once", () => {
    expect(fuzzyFilterProjectPaths(files, "src", 8)).toEqual([
      { kind: "directory", path: "nucleus-canvas/src" },
      { kind: "file", path: "nucleus-canvas/src/App.tsx" },
    ]);
  });

  it("shows directories first before a query is entered", () => {
    expect(fuzzyFilterProjectPaths(files, "", 2)).toEqual([
      { kind: "directory", path: "nucleus-canvas" },
      { kind: "directory", path: "nucleus-canvas/src" },
    ]);
  });
});
