// Output style — a per-turn persona/tone the agent's replies follow. Mirrors
// `REASONING_EFFORTS`'s shape: a small fixed set, not a markdown-file
// convention (see provider-local/src/prompt.rs's `OUTPUT_STYLES`, which this
// list's `id`s must match).

export interface OutputStyleInfo {
  id: string;
  label: string;
  description: string;
}

export const OUTPUT_STYLES: OutputStyleInfo[] = [
  { id: "default", label: "Default", description: "Clark's normal voice." },
  { id: "terse", label: "Terse", description: "Minimal narration — just the work and the result." },
  { id: "teaching", label: "Teaching", description: "Explains reasoning and trade-offs as it works." },
];

export const DEFAULT_OUTPUT_STYLE = "default";

const KEY = "clark-desktop:output-style";

export function loadOutputStyle(): string {
  try {
    const v = localStorage.getItem(KEY);
    if (v && OUTPUT_STYLES.some((s) => s.id === v)) return v;
  } catch {
    /* ignore */
  }
  return DEFAULT_OUTPUT_STYLE;
}

export function saveOutputStyle(style: string): void {
  try {
    localStorage.setItem(KEY, style);
  } catch {
    /* ignore */
  }
}
