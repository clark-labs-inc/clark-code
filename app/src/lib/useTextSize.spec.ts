import { describe, expect, it } from "vitest";
import {
  loadTextSize,
  parseTextSize,
  saveTextSize,
  stepTextSize,
  terminalFontSize,
} from "./useTextSize";

class MemoryStorage {
  values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

describe("text size preference", () => {
  it("uses default for missing or invalid persisted values", () => {
    const storage = new MemoryStorage();
    expect(loadTextSize(storage)).toBe(100);
    storage.setItem("clark.text-size", "enormous");
    expect(loadTextSize(storage)).toBe(100);
  });

  it("persists and reloads a browser-style percentage", () => {
    const storage = new MemoryStorage();
    saveTextSize(150, storage);
    expect(storage.getItem("clark.text-size")).toBe("150");
    expect(loadTextSize(storage)).toBe(150);
  });

  it("migrates the three legacy semantic presets", () => {
    expect(parseTextSize("compact")).toBe(90);
    expect(parseTextSize("default")).toBe(100);
    expect(parseTextSize("large")).toBe(110);
  });

  it("steps through presets and clamps at both ends", () => {
    expect(stepTextSize(75, -1)).toBe(75);
    expect(stepTextSize(75, 1)).toBe(80);
    expect(stepTextSize(90, 1)).toBe(100);
    expect(stepTextSize(100, 1)).toBe(110);
    expect(stepTextSize(110, 1)).toBe(125);
    expect(stepTextSize(200, 1)).toBe(200);
  });

  it("keeps terminal text in sync with the application scale", () => {
    expect(terminalFontSize(75)).toBe(10.5);
    expect(terminalFontSize(100)).toBe(14);
    expect(terminalFontSize(200)).toBe(28);
  });
});
