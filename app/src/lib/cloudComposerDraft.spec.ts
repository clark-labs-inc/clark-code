import { describe, expect, it } from "vitest";
import {
  cloudComposerDraftBaseRevision,
  cloudComposerDraftKey,
  isSubmittedDraftResidue,
} from "./cloudComposerDraft";

describe("cloud composer draft scope", () => {
  it("uses a clean namespace for unbound drafts without changing conversation IDs", () => {
    expect(cloudComposerDraftKey(null)).toBe("new.v3");
    expect(cloudComposerDraftKey("conversation-123")).toBe("conversation-123");
    expect(cloudComposerDraftKey("specialist:spec:new.v3")).toBe("specialist:spec:new.v3");
  });

  it("recreates a server-absent draft from revision zero", () => {
    expect(cloudComposerDraftBaseRevision(null)).toBe(0);
    expect(cloudComposerDraftBaseRevision({
      draftKey: "new.v3",
      text: "remote",
      rev: 7,
      updatedAt: "2026-08-12T00:00:00Z",
    })).toBe(7);
  });

  it("identifies exact and chunk-prefix residue without consuming a newer edit", () => {
    const submitted = "Revise the specification and keep all prior content.";

    expect(isSubmittedDraftResidue(submitted, submitted)).toBe(true);
    expect(isSubmittedDraftResidue("Revise the specification", submitted)).toBe(true);
    expect(isSubmittedDraftResidue("", submitted)).toBe(false);
    expect(isSubmittedDraftResidue("A newer follow-up", submitted)).toBe(false);
  });
});
