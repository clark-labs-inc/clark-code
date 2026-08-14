import { describe, expect, it } from "vitest";
import {
  parentDirectoryReadRoots,
  parentDirectorySuggestions,
} from "./parentDirectoryAutocomplete";

const siblings = [
  { name: "clark", path: "/Users/demo/git/clark" },
  { name: "clark-desktop", path: "/Users/demo/git/clark-desktop" },
  { name: "omnivoicegraph", path: "/Users/demo/git/omnivoicegraph" },
];

describe("parentDirectorySuggestions", () => {
  it("advertises parent browsing from the bare mention menu", () => {
    expect(parentDirectorySuggestions("", siblings)).toEqual([
      { kind: "parent_directory_menu" },
    ]);
  });

  it("expands @../ into concrete sibling folders", () => {
    expect(parentDirectorySuggestions("../", siblings)).toEqual([
      { kind: "parent_directory", path: "../clark", root: "/Users/demo/git/clark" },
      {
        kind: "parent_directory",
        path: "../clark-desktop",
        root: "/Users/demo/git/clark-desktop",
      },
      {
        kind: "parent_directory",
        path: "../omnivoicegraph",
        root: "/Users/demo/git/omnivoicegraph",
      },
      { kind: "parent_directory_picker" },
    ]);
  });

  it("filters the parent list by sibling name", () => {
    expect(parentDirectorySuggestions("../omni", siblings)).toEqual([
      {
        kind: "parent_directory",
        path: "../omnivoicegraph",
        root: "/Users/demo/git/omnivoicegraph",
      },
      { kind: "parent_directory_picker" },
    ]);
  });

  it("keeps a folder-picker escape hatch while sibling discovery loads", () => {
    expect(parentDirectorySuggestions("../", [])).toEqual([
      { kind: "parent_directory_picker" },
    ]);
  });
});

describe("parentDirectoryReadRoots", () => {
  it("admits both selected and manually typed sibling mentions", () => {
    expect(parentDirectoryReadRoots(
      "compare @../clark/ with @/opt/reference/",
      [{ path: "/opt/reference", root: "/opt/reference" }],
      siblings,
    )).toEqual(["/opt/reference", "/Users/demo/git/clark"]);
  });

  it("does not confuse a sibling name with a longer prefix", () => {
    expect(parentDirectoryReadRoots("inspect @../clark-desktop/", [], siblings)).toEqual([
      "/Users/demo/git/clark-desktop",
    ]);
  });
});
