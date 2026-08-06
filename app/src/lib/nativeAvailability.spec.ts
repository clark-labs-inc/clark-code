import { describe, expect, it } from "vitest";
import { probeMcp } from "./mcp";
import { probeSsh } from "./ssh";

describe("desktop-only connection probes", () => {
  it("explains that MCP testing requires the desktop app", async () => {
    await expect(
      probeMcp([{ name: "test", command: "npx", args: [], env: {} }]),
    ).rejects.toThrow("Connection testing is available in the desktop app.");
  });

  it("explains that SSH testing requires the desktop app", async () => {
    await expect(probeSsh("example-host")).rejects.toThrow(
      "SSH testing is available in the desktop app.",
    );
  });
});
