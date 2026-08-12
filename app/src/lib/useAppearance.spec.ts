import { describe, expect, it } from "vitest";
import {
  DEFAULT_INTERFACE_CONTRAST,
  INTERFACE_CONTRASTS,
  loadInterfaceContrast,
  parseInterfaceContrast,
} from "./useAppearance";

describe("interface contrast preference", () => {
  it("has one closed option set with medium as its default", () => {
    expect(INTERFACE_CONTRASTS).toEqual(["low", "medium", "high", "extra-high"]);
    expect(DEFAULT_INTERFACE_CONTRAST).toBe("medium");
  });

  it("loads valid persisted values and normalizes everything else", () => {
    expect(loadInterfaceContrast({ getItem: () => "extra-high" })).toBe("extra-high");
    expect(loadInterfaceContrast({ getItem: () => "unknown" })).toBe("medium");
    expect(parseInterfaceContrast(null)).toBeNull();
  });
});
