import { describe, expect, it } from "vitest";
import { diffStats, langFromPath, parseDiff } from "./diff";

const SAMPLE = `diff --git a/src/lib.rs b/src/lib.rs
index 3f2a1b4..9c8e7d2 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,7 +10,9 @@ pub fn main() {
     let nums = vec![1, 2, 3];
     for n in &nums {
-        println!("n = {}", n);
+        if *n > 1 {
+            println!("big: {}", n);
+        }
     }
 }
`;

describe("diffStats", () => {
  it("counts added/removed lines", () => {
    expect(diffStats(SAMPLE)).toEqual({ adds: 3, dels: 1 });
  });
  it("returns null for non-diff text", () => {
    expect(diffStats("just some output")).toBeNull();
  });
});

describe("parseDiff", () => {
  it("returns null for non-diff text", () => {
    expect(parseDiff("not a diff")).toBeNull();
  });

  it("extracts the path from the +++ line (stripping b/)", () => {
    const f = parseDiff(SAMPLE);
    expect(f?.path).toBe("src/lib.rs");
  });

  it("falls back to the diff --git line path when there's no +++ line", () => {
    const f = parseDiff("diff --git a/x.go b/x.go\n@@ -1 +1 @@\n-a\n+b\n");
    expect(f?.path).toBe("x.go");
  });

  it("classifies lines and tracks old/new line numbers across a hunk", () => {
    const f = parseDiff(SAMPLE)!;
    // First three are meta: diff --git, index, ---, +++
    expect(f.lines[0]).toEqual({ kind: "meta", text: "diff --git a/src/lib.rs b/src/lib.rs" });
    expect(f.lines[1]).toEqual({ kind: "meta", text: "index 3f2a1b4..9c8e7d2 100644" });
    const hunk = f.lines.find((l) => l.kind === "hunk");
    expect(hunk).toEqual({ kind: "hunk", text: "@@ -10,7 +10,9 @@ pub fn main() {" });

    // Hunk starts at old 10 / new 10. First context line is line 10 in both.
    const ctx = f.lines.find((l) => l.kind === "context")!;
    expect(ctx).toMatchObject({ oldNo: 10, newNo: 10, text: "    let nums = vec![1, 2, 3];" });

    const del = f.lines.find((l) => l.kind === "del")!;
    expect(del).toMatchObject({ oldNo: 12, text: '        println!("n = {}", n);' });

    const adds = f.lines.filter((l) => l.kind === "add");
    expect(adds).toHaveLength(3);
    expect(adds[0]).toMatchObject({ newNo: 12, text: "        if *n > 1 {" });
    expect(adds[1]).toMatchObject({ newNo: 13, text: '            println!("big: {}", n);' });
    expect(adds[2]).toMatchObject({ newNo: 14, text: "        }" });

    expect(f.stats).toEqual({ adds: 3, dels: 1 });
  });

  it("handles a no-newline marker line as meta, not content", () => {
    const f = parseDiff("diff --git a/x b/x\n@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+a\n");
    expect(f?.lines.some((l) => l.kind === "meta" && l.text.startsWith("\\ No newline"))).toBe(true);
  });
});

describe("langFromPath", () => {
  it("maps common extensions to a Shiki id", () => {
    expect(langFromPath("src/lib.rs")).toBe("rust");
    expect(langFromPath("app.tsx")).toBe("tsx");
    expect(langFromPath("config.yml")).toBe("yaml");
    expect(langFromPath("run.sh")).toBe("bash");
  });
  it("returns null for unknown / extensionless paths", () => {
    expect(langFromPath("Makefile")).toBeNull();
    expect(langFromPath("weird.xyz")).toBeNull();
  });
});
