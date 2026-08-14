import type { ComposerSuggestion } from "./composerInput";
import { fuzzyScore } from "./fuzzy";

export interface SpecRepositoryChoice {
  path: string;
  current: boolean;
}

/** Repository-level rows for Spec's `@` menu. `@repo` deliberately expands
 * to concrete folders; it is never a dead command that only opens a notice. */
export function specRepositorySuggestions(
  query: string,
  choices: readonly SpecRepositoryChoice[],
  repositoryRoot: string,
): ComposerSuggestion[] {
  const normalized = query.toLowerCase();
  const repoIntent = !normalized
    || "repo".startsWith(normalized)
    || normalized.startsWith("repo");
  const repositoryQuery = normalized.startsWith("repo")
    ? normalized.slice("repo".length).replace(/^[/\\:-]+/, "")
    : normalized;
  const repositories = choices
    .filter((choice) => {
      if (repoIntent && !repositoryQuery) return true;
      const label = choice.path.split(/[\\/]/).filter(Boolean).at(-1) ?? choice.path;
      return Boolean(
        fuzzyScore(repositoryQuery, label)
        || fuzzyScore(repositoryQuery, choice.path),
      );
    })
    .slice(0, 6)
    .map((choice): ComposerSuggestion => ({ kind: "spec_repository", ...choice }));
  return [
    ...repositories,
    ...(repoIntent ? [{ kind: "spec_repository_picker" as const }] : []),
    ...(repositoryRoot && "folder".includes(normalized)
      ? [{ kind: "spec_folder" as const }]
      : []),
  ].slice(0, 8);
}
