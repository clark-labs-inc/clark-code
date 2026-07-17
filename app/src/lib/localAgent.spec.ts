import { describe, expect, it } from "vitest";
import { CODING_MODELS, DEFAULT_LOCAL_SETTINGS, REASONING_EFFORTS, modelLabel } from "./localAgent";

describe("Clark Code model settings", () => {
  it("keeps GLM 5.2 as the default", () => {
    expect(DEFAULT_LOCAL_SETTINGS.model).toBe("clark-code");
    expect(modelLabel(DEFAULT_LOCAL_SETTINGS.model)).toBe("GLM 5.2");
  });

  it("exposes Grok 4.5 and DeepSeek V4 Pro through backend-owned aliases", () => {
    expect(CODING_MODELS.map(({ id, label }) => ({ id, label }))).toEqual([
      { id: "clark-code", label: "GLM 5.2" },
      { id: "clark-code:kimi_k3", label: "Kimi K3" },
      { id: "clark-code:kimi_k27_code", label: "Kimi K2.7 Code" },
      { id: "clark-code:grok45", label: "Grok 4.5" },
      { id: "clark-code:deepseek_v4_pro", label: "DeepSeek V4 Pro" },
    ]);
  });

  it("offers only the portable High and Max reasoning overrides", () => {
    expect(REASONING_EFFORTS.map(({ id }) => id)).toEqual(["", "high", "xhigh"]);
  });
});
