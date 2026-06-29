// A tiny dependency-free fuzzy matcher, shared by the command palette, the
// `@`-file picker, and slash commands. Subsequence match with scoring that
// rewards consecutive runs, word/path-segment boundaries, and earlier matches.

export interface FuzzyMatch<T> {
  item: T;
  score: number;
  /** Indices in the key string that matched — for highlighting. */
  positions: number[];
}

const BOUNDARY = /[/\-_. ]/;

/** Score `query` as a subsequence of `text`. Returns null when `query` isn't a
 *  subsequence. Higher scores are better matches. */
export function fuzzyScore(
  query: string,
  text: string,
): { score: number; positions: number[] } | null {
  if (!query) return { score: 0, positions: [] };
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  const positions: number[] = [];
  let qi = 0;
  let score = 0;
  let prev = -2;
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] !== q[qi]) continue;
    let bonus = 1;
    if (ti === prev + 1) bonus += 4; // consecutive character
    if (ti === 0 || BOUNDARY.test(t[ti - 1])) bonus += 3; // start of a word / path segment
    score += bonus;
    positions.push(ti);
    prev = ti;
    qi++;
  }
  if (qi < q.length) return null; // not all query chars matched
  // Prefer an earlier first hit and a shorter target.
  score -= positions[0] * 0.1;
  score -= text.length * 0.01;
  return { score, positions };
}

/** Fuzzy-rank file paths, preferring matches in the basename — what people
 *  usually mean when they type `@foo`. Falls back to a full-path match so
 *  `@dir/foo` still works. */
export function fuzzyFilterFiles(paths: string[], query: string, limit = 8): string[] {
  if (!query.trim()) return paths.slice(0, limit);
  const out: { path: string; score: number }[] = [];
  for (const path of paths) {
    const base = path.slice(path.lastIndexOf("/") + 1);
    const b = fuzzyScore(query, base);
    const p = fuzzyScore(query, path);
    if (!b && !p) continue;
    // A clean basename hit outranks a scattered full-path one.
    const score = Math.max(b ? b.score + 6 : -Infinity, p ? p.score : -Infinity);
    out.push({ path, score });
  }
  out.sort((a, b) => b.score - a.score);
  return out.slice(0, limit).map((x) => x.path);
}

/** Filter + rank `items` by `query` against `key`. An empty query passes items
 *  through in their original order (capped). */
export function fuzzyFilter<T>(
  items: T[],
  query: string,
  key: (item: T) => string,
  limit = 50,
): FuzzyMatch<T>[] {
  if (!query.trim()) {
    return items.slice(0, limit).map((item) => ({ item, score: 0, positions: [] }));
  }
  const out: FuzzyMatch<T>[] = [];
  for (const item of items) {
    const m = fuzzyScore(query, key(item));
    if (m) out.push({ item, score: m.score, positions: m.positions });
  }
  out.sort((a, b) => b.score - a.score);
  return out.slice(0, limit);
}
