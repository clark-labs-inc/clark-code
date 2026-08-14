import type { ProjectDirectory } from "../core-bridge/bridge";
import type { ComposerSuggestion } from "./composerInput";
import { fuzzyScore } from "./fuzzy";

/** Shared-chat `@` rows for browsing outside the current checkout. The menu
 * advertises the parent boundary at bare `@`; concrete siblings appear only
 * after the user commits to `../` so ordinary file matching stays focused. */
export function parentDirectorySuggestions(
  query: string,
  directories: readonly ProjectDirectory[],
): ComposerSuggestion[] {
  if (!query) return [{ kind: "parent_directory_menu" }];
  if (!/^\.\.[/\\]?/.test(query)) return [];
  const nameQuery = query.replace(/^\.\.[/\\]?/, "");
  const matches = directories
    .flatMap((directory) => {
      const match = fuzzyScore(nameQuery, directory.name)
        ?? fuzzyScore(nameQuery, directory.path);
      return match ? [{ directory, score: match.score }] : [];
    })
    .sort((left, right) => right.score - left.score)
    .slice(0, 7)
    .map(({ directory }): ComposerSuggestion => ({
      kind: "parent_directory",
      path: `../${directory.name}`,
      root: directory.path,
    }));
  return [...matches, { kind: "parent_directory_picker" }];
}

export function parentDirectoryReadRoots(
  message: string,
  selected: readonly { path: string; root: string }[],
  directories: readonly ProjectDirectory[],
): string[] {
  const mentioned = new Set(
    (message.match(/@\S+/g) ?? []).map((mention) => mention
      .slice(1)
      .replace(/[),.;:!?]+$/, "")
      .replace(/[\\/]+$/, "")),
  );
  return [...new Set([
    ...selected
      .filter(({ path }) => message.includes(`@${path}`))
      .map(({ root }) => root),
    ...directories
      .filter(({ name }) => mentioned.has(`../${name}`))
      .map(({ path }) => path),
  ])];
}
