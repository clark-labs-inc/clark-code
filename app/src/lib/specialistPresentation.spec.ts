import { describe, expect, it } from "vitest";

import { SPECIALIST_KINDS } from "./specialists";
import {
  specialistConversationPresentation,
  specialistPresentationFromPayload,
  specialistPresentationPayload,
} from "./specialistPresentation";

describe("specialist conversation presentation examples", () => {
  it("covers every registered first-party specialist with bounded presentation data", () => {
    for (const kind of SPECIALIST_KINDS) {
      const presentation = specialistConversationPresentation(kind);

      expect(presentation?.kind).toBe(kind);
      expect(presentation?.diagram).toContain("flowchart LR");
      expect(presentation?.metrics).toHaveLength(3);
      expect(presentation?.evidence).toHaveLength(3);
      expect(presentation?.stages).toHaveLength(4);
      expect(new Set(presentation?.evidence.map(({ id }) => id)).size).toBe(3);
      expect(new Set(presentation?.stages.map(({ id }) => id)).size).toBe(4);
      expect(
        presentation?.metrics.every(({ progress }) => progress >= 0 && progress <= 100),
      ).toBe(true);
      expect(
        presentation?.evidence.every(({ confidence }) => confidence >= 0 && confidence <= 100),
      ).toBe(true);
    }
  });

  it("keeps internal runtime and provider labels out of user-facing examples", () => {
    const serialized = SPECIALIST_KINDS
      .map((kind) => specialistConversationPresentation(kind))
      .map((presentation) => JSON.stringify(presentation))
      .join("\n");

    expect(serialized).not.toContain("security_scan_contract");
    expect(serialized).not.toContain("delegate_read_only");
    expect(serialized).not.toContain("z-ai/glm");
    expect(serialized).not.toContain("qwen/");
    expect(serialized).not.toContain("provider-specialist");
  });

  it("round-trips the typed conversation payload used by native snapshots", () => {
    const presentation = specialistConversationPresentation("security");
    expect(presentation).not.toBeNull();
    if (!presentation) return;

    const payload = specialistPresentationPayload(presentation);
    expect(payload.diagram_title).toBe(presentation.diagramTitle);
    expect(specialistPresentationFromPayload(payload)).toEqual(presentation);
  });
});
