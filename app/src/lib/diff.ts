import type { ContentBlock, ToolCall } from "../core-bridge/types";

export interface DiffStat {
  adds: number;
  dels: number;
}

/** Flatten a tool result's content blocks to plain text. */
export function blocksToText(blocks: ContentBlock[]): string {
  return blocks.map((b) => (b.type === "text" ? b.text : "")).join("");
}

/** Count added/removed lines in a `diff <path>\n@@…` unified diff (the shape the
 *  local edit/write tools emit). Returns null when the text isn't such a diff.
 *  Skips header lines (`diff`, `index`, `+++`, `---`, `@@`) so only real content
 *  `+`/`-` lines are counted. */
export function diffStats(text: string): DiffStat | null {
  if (!text.startsWith("diff ")) return null;
  let adds = 0;
  let dels = 0;
  for (const line of text.split("\n")) {
    if (line.startsWith("diff ") || line.startsWith("@@")) continue;
    if (line.startsWith("index ") || line.startsWith("similarity ") || line.startsWith("rename ")) continue;
    if (line.startsWith("new file") || line.startsWith("deleted file")) continue;
    if (line.startsWith("old mode") || line.startsWith("new mode")) continue;
    if (line.startsWith("copy from") || line.startsWith("copy to")) continue;
    if (line.startsWith("+++ ") || line.startsWith("--- ")) continue;
    if (line.startsWith("\\")) continue; // "\ No newline at end of file"
    if (line.startsWith("+")) adds++;
    else if (line.startsWith("-")) dels++;
  }
  return adds || dels ? { adds, dels } : null;
}

/** Per-edit stats for a single tool call (null if it isn't a rendered edit). */
export function callDiffStat(call: ToolCall): DiffStat | null {
  if (call.kind !== "edit") return null;
  return diffStats(blocksToText(call.content));
}

/** One line of a parsed unified diff, typed for rendering. */
export type DiffLine =
  | { kind: "meta"; text: string } // `diff --git`, `index`, `+++`, `---`
  | { kind: "hunk"; text: string } // `@@ -a,b +c,d @@ …`
  | { kind: "context"; oldNo: number | null; newNo: number | null; text: string }
  | { kind: "add"; newNo: number; text: string }
  | { kind: "del"; oldNo: number; text: string }
  | { kind: "plain"; text: string }; // anything outside a hunk (e.g. "No changes")

export interface DiffFile {
  /** Display path, derived from the `+++ b/…` line (falls back to the `diff` line). */
  path: string;
  stats: DiffStat;
  lines: DiffLine[];
}

/** Parse a unified diff (`diff --git …\n@@ …`) into structured, renderable
 *  lines with old/new line numbers. Tolerates the `index`/`+++`/`---` header
 *  and multiple hunks. Returns null when the text isn't a `diff `-prefixed diff. */
export function parseDiff(text: string): DiffFile | null {
  if (!text.startsWith("diff ")) return null;
  const rawLines = text.split("\n");
  let path = "";
  let adds = 0;
  let dels = 0;
  const out: DiffLine[] = [];

  // Track running line numbers as we walk hunks.
  let oldNo = 0;
  let newNo = 0;
  let inHunk = false;

  for (let i = 0; i < rawLines.length; i++) {
    const line = rawLines[i];
    if (line.startsWith("diff ")) {
      out.push({ kind: "meta", text: line });
      continue;
    }
    if (line.startsWith("index ") || line.startsWith("similarity ") || line.startsWith("rename ") || line.startsWith("new file") || line.startsWith("deleted file") || line.startsWith("old mode") || line.startsWith("new mode") || line.startsWith("copy from") || line.startsWith("copy to")) {
      out.push({ kind: "meta", text: line });
      continue;
    }
    if (line.startsWith("--- ") || line.startsWith("+++ ")) {
      out.push({ kind: "meta", text: line });
      // Capture the path from `+++ b/path` (strip the `b/` prefix git adds).
      if (line.startsWith("+++ ")) {
        const p = line.slice(4).trim();
        path = p.startsWith("b/") ? p.slice(2) : p.replace(/^\/dev\/null$/, "");
      }
      continue;
    }
    const hunk = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
    if (hunk) {
      inHunk = true;
      oldNo = parseInt(hunk[1], 10);
      newNo = parseInt(hunk[2], 10);
      out.push({ kind: "hunk", text: line });
      continue;
    }
    if (!inHunk) {
      if (line === "") continue; // blank line between header fields
      out.push({ kind: "plain", text: line });
      continue;
    }
    if (line.startsWith("+")) {
      adds++;
      out.push({ kind: "add", newNo: newNo++, text: line.slice(1) });
    } else if (line.startsWith("-")) {
      dels++;
      out.push({ kind: "del", oldNo: oldNo++, text: line.slice(1) });
    } else if (line.startsWith("\\")) {
      // "\ No newline at end of file" — a marker, not a content line.
      out.push({ kind: "meta", text: line });
    } else {
      const text = line.startsWith(" ") ? line.slice(1) : line;
      out.push({ kind: "context", oldNo: oldNo++, newNo: newNo++, text });
    }
  }

  if (!path) {
    // Fall back to the path in the `diff --git a/x b/x` line.
    const m = /^diff --git a\/(\S+) b\/(\S+)/.exec(rawLines[0]);
    path = m ? m[2] : rawLines[0].slice(5);
  }
  return { path, stats: { adds, dels }, lines: out };
}

/** Best-effort Shiki language id from a file path's extension. Returns null for
 *  unknown/extensionless paths so the caller falls back to plain rendering. */
export function langFromPath(path: string): string | null {
  const ext = path.includes(".") ? path.slice(path.lastIndexOf(".") + 1).toLowerCase() : "";
  const map: Record<string, string> = {
    ts: "typescript", mts: "typescript", cts: "typescript",
    tsx: "tsx", js: "javascript", mjs: "javascript", cjs: "javascript",
    jsx: "jsx", rs: "rust", py: "python", rb: "ruby", go: "go",
    json: "json", yml: "yaml", yaml: "yaml", toml: "toml",
    css: "css", html: "html", htm: "html", md: "markdown",
    sh: "bash", bash: "bash", zsh: "bash", sql: "sql",
    vue: "vue", svelte: "svelte", php: "php", java: "java",
    kt: "kotlin", swift: "swift", c: "c", h: "c", cpp: "cpp",
    cc: "cpp", hpp: "cpp", cs: "csharp", scala: "scala",
  };
  return map[ext] ?? null;
}

export interface EditSummary {
  files: number;
  adds: number;
  dels: number;
}

/** Aggregate edit stats across a group of tool calls — the "N files changed,
 *  +X −Y" summary shown under a block of agent work. */
export function summarizeEdits(calls: ToolCall[]): EditSummary | null {
  const byFile = new Map<string, DiffStat>();
  for (const call of calls) {
    const stat = callDiffStat(call);
    if (!stat) continue;
    const path = call.locations?.[0]?.path ?? call.id;
    const prev = byFile.get(path) ?? { adds: 0, dels: 0 };
    byFile.set(path, { adds: prev.adds + stat.adds, dels: prev.dels + stat.dels });
  }
  if (byFile.size === 0) return null;
  let adds = 0;
  let dels = 0;
  for (const s of byFile.values()) {
    adds += s.adds;
    dels += s.dels;
  }
  return { files: byFile.size, adds, dels };
}
