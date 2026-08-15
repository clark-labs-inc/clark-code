const TOKEN_WRAP_MIN_NEWLINES = 4;
const TOKEN_WRAP_MIN_INTERNAL_SPLITS = 2;
// Observed DeepSeek adapters have emitted a leading-space token in the middle
// of `dependencies`; keep this allowlist evidence-based rather than guessing
// across ordinary short words such as `the` or `for`.
const KNOWN_LEADING_SPACE_PREFIXES = new Set(["dep"]);

function countMatches(text: string, pattern: RegExp): number {
  return Array.from(text.matchAll(pattern)).length;
}

function looksTokenWrapped(text: string): boolean {
  return (
    countMatches(text, /\n/g) >= TOKEN_WRAP_MIN_NEWLINES &&
    countMatches(text, /(?<=[\p{L}\p{N}_])\n(?=[\p{L}\p{N}_])/gu) >=
      TOKEN_WRAP_MIN_INTERNAL_SPLITS
  );
}

function repairProse(text: string): string {
  // Some provider adapters put a leading space on a continuation token. Only
  // join that ambiguous boundary when the continuation is itself split again;
  // ordinary prose such as `for\n dependencies` remains two words.
  const joinedContinuations = text.replace(
    /(^|[ \t])([\p{L}]{1,4})\n[ \t]+(?=[\p{L}]+(?:\n[\p{L}]+)+)/gu,
    (boundary, leading: string, prefix: string) =>
      KNOWN_LEADING_SPACE_PREFIXES.has(prefix.toLocaleLowerCase())
        ? `${leading}${prefix}`
        : boundary,
  );

  return joinedContinuations.replace(/\n+/g, (newlines, offset: number, source: string) => {
    if (newlines.length > 1) return newlines;
    const tail = source.slice(offset + 1);
    // Preserve Markdown structure even in a malformed reasoning paragraph.
    if (/^[ \t]*(?:```|~~~|#{1,6}\s|>|[-+*]\s|\d+[.)]\s)/.test(tail)) {
      return "\n";
    }
    return "";
  });
}

/**
 * Derive readable Thinking text from provider output without changing the raw
 * reasoning stored in the trajectory or replayed to the provider.
 */
export function thinkingForDisplay(raw: string): string {
  if (!looksTokenWrapped(raw)) return raw;

  // Fenced code is byte-sensitive presentation content; never reflow it.
  return raw
    .split(/(```[^\n]*\n[\s\S]*?```|~~~[^\n]*\n[\s\S]*?~~~)/g)
    .map((part, index) => (index % 2 === 1 ? part : repairProse(part)))
    .join("");
}
