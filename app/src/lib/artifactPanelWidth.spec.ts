import { describe, expect, it } from "vitest";
import {
  ARTIFACT_PANEL_WIDTH_KEY,
  DEFAULT_ARTIFACT_PANEL_WIDTH,
  MIN_ARTIFACT_PANEL_WIDTH,
  constrainArtifactPanelWidth,
  loadArtifactPanelWidth,
  saveArtifactPanelWidth,
} from "./artifactPanelWidth";

class MemoryStorage {
  private values = new Map<string, string>();

  getItem(key: string) {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string) {
    this.values.set(key, value);
  }
}

describe("artifact panel width", () => {
  it("keeps both panes usable while resizing", () => {
    expect(constrainArtifactPanelWidth(200, 1200)).toBe(MIN_ARTIFACT_PANEL_WIDTH);
    expect(constrainArtifactPanelWidth(700, 1200)).toBe(700);
    expect(constrainArtifactPanelWidth(1100, 1200)).toBe(880);
  });

  it("falls back safely when the split view is narrower than both minimums", () => {
    expect(constrainArtifactPanelWidth(DEFAULT_ARTIFACT_PANEL_WIDTH, 600)).toBe(280);
  });

  it("persists and reloads the last chosen width", () => {
    const storage = new MemoryStorage();
    saveArtifactPanelWidth(733.4, storage);

    expect(storage.getItem(ARTIFACT_PANEL_WIDTH_KEY)).toBe("733");
    expect(loadArtifactPanelWidth(storage)).toBe(733);
  });

  it("ignores invalid persisted values", () => {
    const storage = new MemoryStorage();
    storage.setItem(ARTIFACT_PANEL_WIDTH_KEY, "not-a-width");

    expect(loadArtifactPanelWidth(storage)).toBe(DEFAULT_ARTIFACT_PANEL_WIDTH);
  });
});
