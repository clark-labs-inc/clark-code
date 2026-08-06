// Pull source links out of a research findings blob so they can be shown as
// clickable citation chips.

export interface Source {
  url: string;
  label: string;
}

/** The distinct sites cited in `text`, one chip per hostname (first-mention
 *  order, capped). Deduping by host keeps the row a scannable "sites consulted"
 *  list rather than repeating the same domain for every deep link. */
export function extractSources(text: string, max = 16): Source[] {
  const seenHost = new Set<string>();
  const out: Source[] = [];
  const re = /https?:\/\/[^\s)<>"'`\]]+/g;
  for (const m of text.matchAll(re)) {
    // Trim trailing punctuation that often clings to URLs in prose.
    const url = m[0].replace(/[.,;:!?)\]}'"]+$/, "");
    let host: string;
    try {
      host = new URL(url).hostname.replace(/^www\./, "");
    } catch {
      continue; // not a parseable URL — skip
    }
    if (seenHost.has(host)) continue;
    seenHost.add(host);
    out.push({ url, label: host });
    if (out.length >= max) break;
  }
  return out;
}
