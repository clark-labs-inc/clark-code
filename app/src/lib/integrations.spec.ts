import { describe, expect, it } from "vitest";
import { integrationRequest } from "./integrations";

describe("native integration boundaries", () => {
  it("does not pretend a browser preview has Messages access", async () => {
    await expect(integrationRequest({ action: "catalog" })).rejects.toThrow("desktop app");
  });
});
