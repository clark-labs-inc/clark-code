export const LANG_ALIASES: Record<string, string> = {
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


export function resolveLang(raw: string | undefined): string | null {
  if (!raw) return null;
  const id = raw.trim().toLowerCase().split(/\s+/)[0];
  if (!id) return null;
  return LANG_ALIASES[id] ?? id;
}

export function highlightCacheKey(lang: string | undefined, code: string): string {
  // The separator cannot appear in a language id, so no key is ambiguous.
  return `${lang ?? ""}\u0000${code}`;
}
