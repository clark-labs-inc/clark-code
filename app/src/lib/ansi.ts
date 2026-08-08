// ANSI escape → HTML coloring for streaming shell output, using `ansi_up` with
// `use_classes` so the 16 ANSI colors emit semantic CSS classes (e.g.
// `ansi-red-fg`) rather than inline hex. index.css maps those classes onto the
// paper/graphite + violet tokens, so colored command output (cargo, pytest, rg,
// …) reads like a real terminal in both themes without bringing in a fixed
// palette.

import { AnsiUp } from "ansi_up";

let converter: AnsiUp | null = null;

/** Lazily-built singleton — AnsiUp holds buffer/state, so one instance serves
 *  all conversions (it's stateless across calls once `ansi_to_html` returns). */
function get(): AnsiUp {
  if (!converter) {
    converter = new AnsiUp();
    converter.use_classes = true;
    converter.escape_html = true;
  }
  return converter;
}

/** Convert a string with ANSI color codes to themed HTML spans. Plain text (no
 *  codes) passes through escaped and unchanged. */
export function ansiToHtml(text: string): string {
  return get().ansi_to_html(text);
}
