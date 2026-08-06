import { describe, expect, it } from "vitest";
import {
  DEFAULT_SIDEBAR_WIDTH,
  MIN_SIDEBAR_WIDTH,
  SIDEBAR_WIDTH_KEY,
  constrainSidebarWidth,
  loadSidebarWidth,
  saveSidebarWidth,
} from "./sidebarWidth";

class MemoryStorage {
  private values = new Map<string, string>();

  getItem(key: string) {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string) {
    this.values.set(key, value);
  }
}

describe("resizable sidebar width", () => {
  it("clamps drags to the min/max range but never below the minimum", () => {
    expect(constrainSidebarWidth(120, 1600)).toBe(MIN_SIDEBAR_WIDTH);
    expect(constrainSidebarWidth(DEFAULT_SIDEBAR_WIDTH, 1600)).toBe(DEFAULT_SIDEBAR_WIDTH);
    expect(constrainSidebarWidth(500, 1600)).toBe(500);
  });

  it("leaves room for the conversation pane on smaller windows", () => {
    // 800 - 360 reserved = 440 max, so a stored 640 collapses to 440.
    expect(constrainSidebarWidth(640, 800)).toBe(440);
    expect(constrainSidebarWidth(320, 560)).toBe(200);
  });

  it("falls back to the historical width when the window is unknown", () => {
    expect(constrainSidebarWidth(DEFAULT_SIDEBAR_WIDTH, undefined)).toBe(DEFAULT_SIDEBAR_WIDTH);
    expect(constrainSidebarWidth(500, undefined)).toBe(500);
  });

  it("persists and reloads the last chosen width, rounded", () => {
    const storage = new MemoryStorage();
    saveSidebarWidth(412.6, storage);

    expect(storage.getItem(SIDEBAR_WIDTH_KEY)).toBe("413");
    expect(loadSidebarWidth(storage)).toBe(413);
  });

  it("ignores invalid persisted values", () => {
    const storage = new MemoryStorage();
    storage.setItem(SIDEBAR_WIDTH_KEY, "not-a-width");

    expect(loadSidebarWidth(storage)).toBe(DEFAULT_SIDEBAR_WIDTH);
  });
});
