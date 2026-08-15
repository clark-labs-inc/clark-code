import { describe, expect, it } from "vitest";
import { newProjectDialogKeyboardIntent } from "./newProjectDialog";

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
