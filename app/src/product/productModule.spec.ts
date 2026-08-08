import { afterEach, describe, expect, it } from "vitest";
import {
  installProductModule,
  neutralProduct,
  productModule,
} from "./productModule";

describe("product module", () => {
  afterEach(() => installProductModule(neutralProduct));

  it("defaults to the neutral open-source composition", () => {
    expect(productModule().branding.id).toBe("desktop");
    expect(productModule().authRequired).toBe(false);
  });

  it("installs a compile-time branded composition", () => {
    installProductModule({
      branding: { id: "example_product", name: "Example", shortName: "Ex" },
      authRequired: true,
      slots: {},
      localAgent: neutralProduct.localAgent,
      artifacts: neutralProduct.artifacts,
      errors: neutralProduct.errors,
    });
    expect(productModule().branding.name).toBe("Example");
    expect(productModule().authRequired).toBe(true);
  });

  it("rejects unsafe product identifiers", () => {
    expect(() => installProductModule({
      branding: { id: "../private", name: "Bad", shortName: "Bad" },
      authRequired: false,
      slots: {},
      localAgent: neutralProduct.localAgent,
      artifacts: neutralProduct.artifacts,
      errors: neutralProduct.errors,
    })).toThrow("invalid");
  });
});
