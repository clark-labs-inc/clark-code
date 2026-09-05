import { describe, expect, it } from "vitest";
import { newProjectDialogKeyboardIntent, remoteFolderAfterHostRefresh } from "./newProjectDialog";

describe("newProjectDialogKeyboardIntent", () => {
  it("closes on Escape and keeps Tab navigation inside the modal", () => {
    expect(newProjectDialogKeyboardIntent("Escape")).toBe("close");
    expect(newProjectDialogKeyboardIntent("Tab")).toBe("cycle_focus");
  });

  it("leaves ordinary keys to the active control", () => {
    expect(newProjectDialogKeyboardIntent("Enter")).toBe("none");
    expect(newProjectDialogKeyboardIntent("ArrowDown")).toBe("none");
  });
});

describe("remote folder refresh", () => {
  it("preserves a custom or deliberately cleared folder on the same host", () => {
    expect(remoteFolderAfterHostRefresh("/work/custom", true, "/work/default")).toBe("/work/custom");
    expect(remoteFolderAfterHostRefresh("", true, "/work/default")).toBe("");
  });
  it("uses the new host's default after the selected host is removed", () => {
    expect(remoteFolderAfterHostRefresh("/old/project", false, "/new/project")).toBe("/new/project");
  });
});
