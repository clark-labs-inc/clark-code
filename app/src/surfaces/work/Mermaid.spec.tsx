import { describe, expect, it } from "vitest";
import { MERMAID_CONFIG } from "./Mermaid";

describe("Mermaid security boundary", () => {
  it("sanitizes conversation-authored SVG before inline rendering", () => {
    expect(MERMAID_CONFIG.securityLevel).toBe("strict");
    expect(MERMAID_CONFIG.securityLevel).not.toBe("loose");
  });
});
