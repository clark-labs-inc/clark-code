import { describe, expect, it } from "vitest";
import { hotkeyBlocked } from "./hotkeys";

describe("global shortcut ownership", () => {
  it("does not run a shortcut already consumed by a control", () => {
    expect(hotkeyBlocked(true, "Tab", false)).toBe(true);
    expect(hotkeyBlocked(true, "n", false)).toBe(true);
  });
  it("keeps modal Tab traversal from changing background approval policy", () => {
    expect(hotkeyBlocked(false, "Tab", true)).toBe(true);
    expect(hotkeyBlocked(false, "Tab", false)).toBe(false);
    expect(hotkeyBlocked(false, "+", true)).toBe(false);
  });
});
