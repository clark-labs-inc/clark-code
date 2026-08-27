import { describe, expect, it } from "vitest";
import { modelCatalog } from "./modelCatalog";

describe("modelCatalog (YAML-driven)", () => {
  it("loads the neutral model catalog from models.yaml", () => {
    expect(modelCatalog.defaultModel).toBe("local-model");
    expect(modelCatalog.models.map(({ id, label }) => ({ id, label }))).toEqual([
      { id: "local-model", label: "Local model" },
      { id: "local-model-large", label: "Large local model" },
    ]);
  });

  it("declares the default model and effort from config", () => {
    expect(modelCatalog.defaultReasoningEffort).toBe("high");
    const local = modelCatalog.models.find((m) => m.id === "local-model");
    expect(local?.defaultReasoningEffort).toBe("high");
  });

});
