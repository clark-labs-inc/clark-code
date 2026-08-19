import type { Artifact } from "../core-bridge/types";

const EMPTY_TITLES = new Set([
  "",
  "new conversation",
  "conversation",
  "new spec",
  "untitled feature",
]);

export interface SpecCodeReference {
  kind: "file" | "folder";
  path: string;
}

function normalizedPath(value: string): string {
  const normalized = value.trim().replace(/\\/g, "/").replace(/\/+$/, "");
  return /^[a-z]:\//i.test(normalized) ? normalized.toLowerCase() : normalized;
}

export function specPathWithinRepository(repositoryRoot: string, candidate: string): boolean {
  const root = normalizedPath(repositoryRoot);
  const path = normalizedPath(candidate);
  return Boolean(root && path && (path === root || path.startsWith(`${root}/`)));
}

export function specRelativePath(repositoryRoot: string, candidate: string): string | null {
  if (!specPathWithinRepository(repositoryRoot, candidate)) return null;
  const root = normalizedPath(repositoryRoot);
  const path = normalizedPath(candidate);
  return path === root ? "." : path.slice(root.length + 1);
}

export function specRepositoryLabel(repositoryRoot: string): string {
  const clean = repositoryRoot.trim().replace(/[\\/]+$/, "");
  return clean.split(/[\\/]/).filter(Boolean).at(-1) ?? clean;
}

export function specCodeContextPrompt(
  message: string,
  repositoryRoot: string,
  references: readonly SpecCodeReference[],
): string {
  if (references.length === 0) return message.trim();
  const context = {
    repository_root: repositoryRoot.trim(),
    references: references.map((reference) => ({
      kind: reference.kind,
      path: reference.path,
    })),
  };
  const request = message.trim()
    || "Read the referenced code and use what you learn to improve the living specification.";
  return `${request}

Continue the feature-specification workflow for the current SPEC.md.

The user attached repository context to this turn:
<spec_code_context>
${JSON.stringify(context, null, 2)}
</spec_code_context>

Inspect the referenced files or folders before changing the specification. Treat the code as evidence of current behavior, not permission to implement anything. If the code and the user's desired behavior differ, preserve that distinction in the specification.

The text at the start of this message is the user's request. Keep it as the conversational title; do not expose the context envelope in user-facing copy.`;
}

export function specDisplayTitle(title?: string | null): string {
  const clean = title?.trim() ?? "";
  return EMPTY_TITLES.has(clean.toLowerCase()) ? "Untitled feature" : clean;
}

/** The living document is the title authority for Spec. Conversation titles
 *  begin as the user's prompt, but the completed document may make a more
 *  precise naming decision. Project that first H1 back into navigation and
 *  downloads so a saved Spec can be found by its actual title after restart. */
export function specDocumentTitle(markdown: string): string | null {
  const heading = markdown
    .split("\n")
    .map((line) => /^#\s+(.+?)\s*#*\s*$/.exec(line)?.[1]?.trim() ?? "")
    .find(Boolean);
  if (!heading) return null;
  const normalized = heading
    .replace(/^(?:feature\s+)?spec(?:ification)?\s*[:\-–—]\s*/i, "")
    .replace(/\s*[:\-–—]\s*(?:(?:product|engineering|feature)\s+)?spec(?:ification)?\s*$/i, "")
    .trim();
  if (!normalized || EMPTY_TITLES.has(normalized.toLowerCase())) return null;
  return normalized;
}

export function specFileStem(title?: string | null): string {
  const display = specDisplayTitle(title)
    .replace(/\b(?:feature\s+)?spec(?:ification)?\b/gi, " ")
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^a-zA-Z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .toLowerCase()
    .slice(0, 96)
    .replace(/-+$/g, "");
  return display || "untitled-feature";
}

export function specFilename(title: string | null | undefined, format: "md" | "pdf"): string {
  return `${specFileStem(title)}_SPEC.${format}`;
}

export function latestSpecArtifact(artifacts: readonly Artifact[]): Artifact | null {
  const markdown = artifacts.filter((artifact) => {
    const value = `${artifact.title} ${artifact.uri ?? ""} ${artifact.mime_type ?? ""}`.toLowerCase();
    return artifact.mime_type === "text/markdown" || /\.(?:md|markdown|mdx)(?:[?#]|\s|$)/.test(value);
  });
  return [...markdown].reverse().find((artifact) => /(?:^|[_\s-])spec(?:\.|[_\s-]|$)/i.test(
    `${artifact.title} ${artifact.uri ?? ""}`,
  )) ?? markdown.at(-1) ?? null;
}

/** A new Spec starts visually empty. The canonical skill grows only sections
 * backed by the user's decisions, so the UI must not present template prompts
 * as if they were already part of the generated document. */
export function initialSpecMarkdown(_title?: string | null): string {
  return "";
}

/** Seed the conversation workspace with one canonical document before the
 * model starts. The comment is invisible in the editor while still giving the
 * persistence boundary non-empty Markdown. */
export function initialSpecDocument(prompt: string): { filename: string; markdown: string } {
  return {
    filename: specFilename(prompt, "md"),
    markdown: "<!-- Spec draft -->\n",
  };
}

/** Tell the Spec workflow which host-prepared file is authoritative. Keeping
 * this after the user's own copy lets prompt history and titles retain only
 * what the person wrote. */
export function preparedSpecDocumentPrompt(message: string, filename: string): string {
  return `${message.trim()}

Continue the feature-specification workflow for the current SPEC.md.

The host has already prepared the only canonical document for this conversation:
<spec_document>
${JSON.stringify({ filename }, null, 2)}
</spec_document>

Read this exact file once and update it directly. Do not search, glob, list directories, or run shell commands to discover another Spec document.`;
}

export interface ScopedSpecPromptDetails {
  selection: string;
  section: string;
  comment: string;
}

function scopedPromptField(text: string, field: string): string {
  return text.match(new RegExp(`<${field}>\\s*([\\s\\S]*?)\\s*</${field}>`, "i"))?.[1]?.trim() ?? "";
}

export function parseScopedSpecPrompt(text: string): ScopedSpecPromptDetails | null {
  const selection = scopedPromptField(text, "selected_spec_content");
  const comment = scopedPromptField(text, "scoped_comment");
  if (!selection || !comment) return null;
  return {
    selection,
    section: scopedPromptField(text, "selected_spec_section") || selection.slice(0, 80),
    comment,
  };
}

export function scopedSpecPrompt(selection: string, question: string, section = selection.slice(0, 80)): string {
  return `Continue the feature-specification workflow for the current SPEC.md.

The user is discussing this document section:
<selected_spec_section>
${section.trim()}
</selected_spec_section>

The user selected this exact document content:
<selected_spec_content>
${selection.trim()}
</selected_spec_content>

Their scoped comment is:
<scoped_comment>
${question.trim()}
</scoped_comment>

Discuss only this selection. Resolve ambiguity with one concise question when necessary; otherwise update the corresponding section of the existing SPEC.md in place. Preserve unrelated sections and the semantic *_SPEC.md filename. Summarize the exact document change after applying it.`;
}
