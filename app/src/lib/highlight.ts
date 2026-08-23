// Syntax highlighting for code blocks via Shiki, using the pure-JS regex engine
// (`@shikijs/engine-javascript`) so no Oniguruma `.wasm` ships with the app —
// important for the Tauri WebView (WKWebView / WebView2), where a bundled WASM
// asset means an async init dance and a heavier load.
//
// We import from the fine-grained subpaths (`shiki/core`, `shiki/langs`,
// `shiki/themes`, `shiki/engine/javascript`) rather than the `shiki` barrel,
// which re-exports the oniguruma engine and would drag its embedded-WASM module
// into the bundle. Vite code-splits each language grammar into its own chunk,
// loaded on demand.
//
// One long-lived core highlighter is created lazily and warmed at module load.
// Code blocks call `highlight()` (async) and render plain monospace until the
// result lands, then upgrade in place.

import { createHighlighterCore, type HighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";
import { bundledLanguages } from "shiki/langs";
import { bundledThemes } from "shiki/themes";

/** Languages an agent streams most often — preloaded so the common case is fast.
 *  Anything else is auto-loaded by the core's `codeToHtml` on first use. */
const COMMON_LANGS = [
  "typescript",
  "tsx",
  "javascript",
  "jsx",
  "bash",
  "shell",
  "rust",
  "json",
  "yaml",
  "toml",
  "python",
  "css",
  "html",
  "markdown",
  "diff",
  "go",
  "sql",
  "regex",
] as const;

/** Dual themes: Shiki emits `--shiki-light` / `--shiki-dark` token variables and
 *  base vars; index.css maps them onto the paper/graphite surfaces and swaps
 *  with `html.dark`. GitHub light/dark reads cleanly on both. */
const LIGHT_THEME = "github-light";
const DARK_THEME = "github-dark";

/** `lang` strings the model emits that aren't Shiki ids — handed to the core as
 *  `langAlias` so it resolves them without a manual map at every call. */
const LANG_ALIASES: Record<string, string> = {
  sh: "bash",
  zsh: "bash",
  py: "python",
  ts: "typescript",
  js: "javascript",
  rs: "rust",
  yml: "yaml",
  md: "markdown",
  plaintext: "markdown",
  text: "markdown",
  txt: "markdown",
};

let core: HighlighterCore | null = null;
let warm: Promise<HighlighterCore> | null = null;

/** Create (once) and return the singleton core highlighter. Preloads the common
 *  languages + both themes with the JS regex engine. */
function getCore(): Promise<HighlighterCore> {
  if (core) return Promise.resolve(core);
  if (warm) return warm;
  warm = createHighlighterCore({
    engine: createJavaScriptRegexEngine(),
    themes: [bundledThemes[LIGHT_THEME], bundledThemes[DARK_THEME]],
    langs: COMMON_LANGS.map((l) => bundledLanguages[l]),
    // Pass a copy: the registry writes grammar aliases (e.g. `sh` →
    // `shellscript`) into this object as languages load, and we don't want it
    // mutating the module-level map that `resolveLang` reads.
    langAlias: { ...LANG_ALIASES },
  }).then((c) => {
    core = c;
    return c;
  });
  return warm;
}

// Warm at module load so the first code block that renders highlights promptly.
// Never throws — a failed warm just falls back to plain rendering.
void getCore().catch(() => {
  warm = null;
});

export interface HighlightResult {
  /** Highlighted HTML (Shiki's `<pre>`), or null if not ready / unknown. */
  html: string | null;
  /** The resolved Shiki language id, or null when the lang is absent/plain. */
  lang: string | null;
}

/** Resolve a raw info-string lang (`ts`, `rust`, `sh`, `text`, …) to a Shiki
 *  language id, or null when it's absent / plainly text. */
export function resolveLang(raw: string | undefined): string | null {
  if (!raw) return null;
  const id = raw.trim().toLowerCase().split(/\s+/)[0];
  if (!id) return null;
  return LANG_ALIASES[id] ?? id;
}

/** Resolve + load a grammar on the warm core, returning the core (or null if
 *  the lang is absent / unknown / the grammar can't load). Shared by the block
 *  and per-line highlighters. */
async function coreForLang(rawLang: string | undefined): Promise<{ core: HighlighterCore; lang: string } | null> {
  const lang = resolveLang(rawLang);
  if (!lang) return null;
  const c = await getCore();
  if (!c.getLoadedLanguages().includes(lang)) {
    const loader = bundledLanguages[lang as keyof typeof bundledLanguages];
    if (!loader) return null;
    try {
      await c.loadLanguage(loader);
    } catch {
      return null;
    }
  }
  return { core: c, lang };
}

/** Bounded memo for tokenized output.
 *
 *  Tokenizing is the single most expensive thing the transcript does — a
 *  60-line TypeScript block measures ~30 ms of main-thread regex work, against
 *  a 16.7 ms frame budget. Streaming re-renders the same block many times as it
 *  grows, and scrolling remounts blocks that were already tokenized once, so
 *  the same input arrives repeatedly. Keyed on language plus exact source, both
 *  of which fully determine the output.
 *
 *  Insertion-ordered eviction (a `Map` iterates in insertion order) keeps this
 *  to a few lines; strict LRU would need a touch-on-read reorder for a workload
 *  that is dominated by recency anyway. */
const HIGHLIGHT_CACHE_ENTRIES = 64;

export function highlightCacheKey(lang: string | undefined, code: string): string {
  // The separator cannot appear in a language id, so no key is ambiguous.
  return `${lang ?? ""}\u0000${code}`;
}

function memoize<T>(limit: number) {
  const entries = new Map<string, T>();
  return {
    get: (key: string) => entries.get(key),
    has: (key: string) => entries.has(key),
    set: (key: string, value: T) => {
      if (entries.has(key)) entries.delete(key);
      entries.set(key, value);
      if (entries.size > limit) {
        const oldest = entries.keys().next();
        if (!oldest.done) entries.delete(oldest.value);
      }
    },
    get size() {
      return entries.size;
    },
    clear: () => entries.clear(),
  };
}

const htmlCache = memoize<HighlightResult>(HIGHLIGHT_CACHE_ENTRIES);
const lineCache = memoize<string[] | null>(HIGHLIGHT_CACHE_ENTRIES);

/** Test seam: the caches are process-global, so a spec must be able to reset. */
export function clearHighlightCaches(): void {
  htmlCache.clear();
  lineCache.clear();
}

/** Highlight `code` for `lang` with both themes (CSS-variable dual theme).
 *  Loads the grammar on demand if it isn't preloaded. Returns `{ html: null }`
 *  while the highlighter warms, or for an absent/plain language — callers render
 *  plain monospace in that case. */
export async function highlight(code: string, rawLang: string | undefined): Promise<HighlightResult> {
  const key = highlightCacheKey(rawLang, code);
  const cached = htmlCache.get(key);
  if (cached) return cached;
  const got = await coreForLang(rawLang);
  if (!got) return { html: null, lang: null };
  try {
    const html = await got.core.codeToHtml(code, {
      lang: got.lang,
      themes: { light: LIGHT_THEME, dark: DARK_THEME },
    });
    const result = { html, lang: got.lang };
    htmlCache.set(key, result);
    return result;
  } catch {
    return { html: null, lang: null };
  }
}

/** Highlight `code` and return one HTML string per source line — the inner
 *  content of each Shiki `<span class="line">`, in arrival order. Used by the
 *  diff renderer to color a file's code within per-row tinted lines. Returns
 *  null while warming or for an unknown lang; callers fall back to plain text. */
export async function highlightLines(code: string, rawLang: string | undefined): Promise<string[] | null> {
  const key = highlightCacheKey(rawLang, code);
  if (lineCache.has(key)) return lineCache.get(key) ?? null;
  const got = await coreForLang(rawLang);
  if (!got) return null;
  let html: string;
  try {
    html = await got.core.codeToHtml(code, {
      lang: got.lang,
      themes: { light: LIGHT_THEME, dark: DARK_THEME },
    });
  } catch {
    return null;
  }
  const lines = splitShikiLineSpans(html, code);
  lineCache.set(key, lines);
  return lines;
}

/** Split Shiki's `<pre><code><span class="line">…</span>…</code></pre>` into one
 *  inner-HTML string per source line. Returns null on a structure mismatch so
 *  the caller can fall back to plain (unhighlighted) text. */
function splitShikiLineSpans(html: string, code: string): string[] | null {
  const lines = code.split("\n");
  // Capture the inner HTML of each <span class="line">…</span>.
  const re = /<span class="line">(.*?)<\/span>/gs;
  const matches = [...html.matchAll(re)];
  if (matches.length === lines.length) return matches.map((m) => m[1] ?? "");
  return null;
}
