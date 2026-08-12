import { beforeEach, describe, expect, it, vi } from "vitest";

const product = vi.hoisted(() => ({ request: vi.fn() }));

vi.mock("../product/productBridge", () => ({
  productRequest: product.request,
}));

import { clearSubmittedCloudComposerDraft } from "./cloudComposerDraft";

const creds = { accountScope: "account-one" };
const submitted = "Write the complete specification.";

function draft(text: string, rev: number) {
  return {
    draftKey: "specialist:spec:new.v3",
    text,
    rev,
    updatedAt: `2026-08-12T00:00:0${rev}.000Z`,
  };
}

describe("cloud composer draft conflict handling", () => {
  beforeEach(() => product.request.mockReset());

  it("clears the accepted current revision", async () => {
    product.request
      .mockResolvedValueOnce(draft(submitted, 4))
      .mockResolvedValueOnce(draft("", 5));

    await expect(clearSubmittedCloudComposerDraft(
      creds,
      "specialist:spec:new.v3",
      submitted,
    )).resolves.toEqual({ outcome: "cleared", draft: draft("", 5) });
    expect(product.request).toHaveBeenCalledTimes(2);
  });

  it("preserves an unrelated edit returned by a revision conflict", async () => {
    const newer = draft("A different device's newer idea", 5);
    product.request
      .mockResolvedValueOnce(draft(submitted, 4))
      .mockResolvedValueOnce({ conflict: true, current: newer });

    await expect(clearSubmittedCloudComposerDraft(
      creds,
      "specialist:spec:new.v3",
      submitted,
    )).resolves.toEqual({ outcome: "preserved_newer", draft: newer });
    expect(product.request).toHaveBeenCalledTimes(2);
  });

  it("bounds a service that rejects its own current revision", async () => {
    const current = draft(submitted, 4);
    product.request
      .mockResolvedValueOnce(current)
      .mockResolvedValueOnce({ conflict: true, current })
      .mockResolvedValueOnce({ conflict: true, current });

    await expect(clearSubmittedCloudComposerDraft(
      creds,
      "specialist:spec:new.v3",
      submitted,
    )).rejects.toThrow("did not accept its current revision");
    expect(product.request).toHaveBeenCalledTimes(3);
  });

});
