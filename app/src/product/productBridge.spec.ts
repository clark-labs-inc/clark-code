import { describe, expect, it } from "vitest";
import { productRequest } from "./productBridge";

describe("product bridge", () => {
  it("rejects unsafe operations before native IPC", async () => {
    await expect(productRequest("../private-operation")).rejects.toThrow("invalid");
    await expect(productRequest("Uppercase")).rejects.toThrow("invalid");
  });
});
