import { describe, expect, it } from "vitest";
import { riskTone } from "./PermissionGate";

describe("riskTone", () => {
  it("names billed image generation without calling it an MCP tool", () => {
    expect(riskTone("billed")?.label).toBe("Billed image");
    expect(riskTone("external")?.label).toBe("External access");
  });

  it("distinguishes network and host-sandbox boundaries", () => {
    expect(riskTone("network")?.label).toBe("Network access");
    expect(riskTone("sandbox")?.label).toBe("Outside sandbox");
  });
});
