import { describe, expect, it } from "vitest";
import {
  loadTextSize,
  saveTextSize,
  stepTextSize,
  TEXT_SIZE_PERCENTAGES,
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
    expect(loadTextSize(storage)).toBe("default");
    storage.setItem("clark.text-size", "enormous");
    expect(loadTextSize(storage)).toBe("default");
  });

  it("persists and reloads a preset", () => {
    const storage = new MemoryStorage();
    saveTextSize("large", storage);
    expect(loadTextSize(storage)).toBe("large");
  });

  it("steps through presets and clamps at both ends", () => {
    expect(stepTextSize("compact", -1)).toBe("compact");
    expect(stepTextSize("compact", 1)).toBe("default");
    expect(stepTextSize("default", 1)).toBe("large");
    expect(stepTextSize("large", 1)).toBe("large");
  });

  it("provides browser-style percentages for shortcut feedback", () => {
    expect(TEXT_SIZE_PERCENTAGES).toEqual({ compact: 90, default: 100, large: 110 });
  });
});
