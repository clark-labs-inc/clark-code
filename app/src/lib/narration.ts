// Split assistant text into spans of plain answer, `<narrate>` commentary, and
// `<thinking>` reasoning. Clark (and some ACP agents) embed these tags inline in
// the message stream; we render each kind differently (answer = markdown,
// narrate = running commentary, thinking = collapsible). Tolerant of unclosed
// tags so it works mid-stream.

export type SpanKind = "text" | "narrate" | "thinking";
export interface NarrationSpan {
  kind: SpanKind;
  text: string;
}

const OPEN = /<(narrate|narration|thinking|think)>/i;

function kindOf(tag: string): SpanKind {
  const t = tag.toLowerCase();
  return t === "thinking" || t === "think" ? "thinking" : "narrate";
}

function push(spans: NarrationSpan[], kind: SpanKind, text: string) {
  if (!text) return;
  const last = spans[spans.length - 1];
  if (last && last.kind === kind) last.text += text;
  else spans.push({ kind, text });
}

export function parseNarration(input: string): NarrationSpan[] {
  const spans: NarrationSpan[] = [];
  let rest = input;
  while (rest.length > 0) {
    const m = rest.match(OPEN);
    if (!m || m.index === undefined) {
      push(spans, "text", rest);
      break;
    }
    if (m.index > 0) push(spans, "text", rest.slice(0, m.index));
    const tag = m[1].toLowerCase();
    const kind = kindOf(tag);
    const after = rest.slice(m.index + m[0].length);
    const close = new RegExp(`</${tag}>`, "i");
    const cm = after.match(close);
    if (cm && cm.index !== undefined) {
      push(spans, kind, after.slice(0, cm.index));
      rest = after.slice(cm.index + cm[0].length);
    } else {
      // Unclosed tag (still streaming) — treat the remainder as this kind.
      push(spans, kind, after);
      break;
    }
  }
  // Trim whitespace edges per span for tidy rendering.
  return spans
    .map((s) => ({ kind: s.kind, text: s.text.replace(/^\s+|\s+$/g, "") }))
    .filter((s) => s.text.length > 0);
}
