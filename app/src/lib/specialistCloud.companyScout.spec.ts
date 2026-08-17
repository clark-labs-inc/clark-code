import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const product = vi.hoisted(() => ({ request: vi.fn() }));

vi.mock("../product/productBridge", () => ({
  productRequest: product.request,
}));

import {
  companyScoutMap,
  specialistSetupCompanyScout,
  type CompanyScoutMap,
} from "./specialistCloud";

const organizationId = "11111111-1111-4111-8111-111111111111";
const credentials = { accountScope: "account-one" };

function map(id: string, updatedAt: number): CompanyScoutMap {
  return {
    id,
    organization_id: organizationId,
    stable_key: "company-scout",
    display_name: "Clark Labs Scout",
    status: "active",
    latest_change_sequence: 0,
    source_count: 0,
    active_machine_count: 0,
    run_count: 0,
    simulation_count: 0,
    updated_at_ms: updatedAt,
  };
}

describe("Company Scout map resolution", () => {
  it("preserves the exact map bound to an existing conversation", () => {
    const current = map("current-map", 2);
    const historical = map("historical-map", 1);
    expect(companyScoutMap([current, historical], historical.id)).toBe(historical);
  });

  it("uses the server's current company map for a new conversation", () => {
    const current = map("current-map", 2);
    expect(companyScoutMap([current, map("historical-map", 1)])).toBe(current);
    expect(companyScoutMap([])).toBeNull();
  });
});

describe("Company Scout setup", () => {
  beforeEach(() => {
    product.request.mockReset();
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  });

  afterEach(() => vi.unstubAllGlobals());

  it("derives the single map name from the selected company", async () => {
    product.request.mockResolvedValue({
      id: "22222222-2222-4222-8222-222222222222",
      organization_id: organizationId,
      stable_key: "company-scout",
      display_name: "Clark Labs Scout",
      coordinator_public_key: "public-key",
    });

    await expect(specialistSetupCompanyScout(
      credentials,
      organizationId,
      "Clark Labs",
    )).resolves.toMatchObject({
      id: "22222222-2222-4222-8222-222222222222",
      organization_id: organizationId,
      stable_key: "company-scout",
      display_name: "Clark Labs Scout",
      status: "active",
    });
    expect(product.request).toHaveBeenCalledWith("specialist.setup_company_scout", {
      organizationId,
      displayName: "Clark Labs Scout",
    });
  });

  it("rejects a setup receipt bound to another company", async () => {
    product.request.mockResolvedValue({
      id: "22222222-2222-4222-8222-222222222222",
      organization_id: "33333333-3333-4333-8333-333333333333",
      stable_key: "company-scout",
      display_name: "Clark Labs Scout",
      coordinator_public_key: "public-key",
    });

    await expect(specialistSetupCompanyScout(
      credentials,
      organizationId,
      "Clark Labs",
    )).rejects.toThrow("invalid Company Scout setup");
  });
});
