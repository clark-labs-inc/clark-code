import { describe, expect, it } from "vitest";
import { specRepositorySuggestions } from "./specRepositoryAutocomplete";

const choices = [
  { path: "/Users/stan/Documents/git/clark-desktop", current: true },
  { path: "/Users/stan/Documents/git/clark", current: false },
  { path: "/Users/stan/Documents/git/clark-public-evals", current: false },
];

describe("specRepositorySuggestions", () => {
  it("expands @repo into actionable current and sibling folders plus the picker", () => {
    expect(specRepositorySuggestions("repo", choices, "")).toEqual([
      { kind: "spec_repository", ...choices[0] },
      { kind: "spec_repository", ...choices[1] },
      { kind: "spec_repository", ...choices[2] },
      { kind: "spec_repository_picker" },
    ]);
  });

  it("autocompletes a sibling repository by name", () => {
    expect(specRepositorySuggestions("public", choices, "")).toEqual([
      { kind: "spec_repository", ...choices[2] },
    ]);
  });

  it("offers folder selection once a repository is focused", () => {
    expect(specRepositorySuggestions("folder", choices, choices[0].path)).toEqual([
      { kind: "spec_folder" },
    ]);
  });
});
