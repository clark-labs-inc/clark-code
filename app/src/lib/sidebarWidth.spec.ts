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
    expect(constrainSidebarWidth(500, 1600)).toBe(400);
  });

  it("leaves room for the conversation pane on smaller windows", () => {
    // A wide persisted sidebar stays within the compact 400px cap.
    expect(constrainSidebarWidth(640, 800)).toBe(400);
    expect(constrainSidebarWidth(320, 560)).toBe(200);
  });

  it("falls back to the historical width when the window is unknown", () => {
    expect(constrainSidebarWidth(DEFAULT_SIDEBAR_WIDTH, undefined)).toBe(DEFAULT_SIDEBAR_WIDTH);
    expect(constrainSidebarWidth(500, undefined)).toBe(400);
  });

  it("persists and reloads the last chosen width, rounded", () => {
    const storage = new MemoryStorage();
    saveSidebarWidth(312.6, storage);

    expect(storage.getItem(SIDEBAR_WIDTH_KEY)).toBe("313");
    expect(loadSidebarWidth(storage)).toBe(313);
  });

  it("ignores invalid persisted values", () => {
    const storage = new MemoryStorage();
    storage.setItem(SIDEBAR_WIDTH_KEY, "not-a-width");

    expect(loadSidebarWidth(storage)).toBe(DEFAULT_SIDEBAR_WIDTH);
  });
});
