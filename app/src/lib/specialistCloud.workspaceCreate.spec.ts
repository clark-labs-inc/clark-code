import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const product = vi.hoisted(() => ({ request: vi.fn() }));

vi.mock("../product/productBridge", () => ({
  productRequest: product.request,
}));

import { specialistCreateWorkspace } from "./specialistCloud";

const organizationId = "11111111-1111-4111-8111-111111111111";
const credentials = { accountScope: "account-one" };

describe("Scout workspace creation", () => {
  beforeEach(() => {
    product.request.mockReset();
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  });

  afterEach(() => vi.unstubAllGlobals());

  it("projects the server create receipt into an immediately selectable workspace", async () => {
    product.request.mockResolvedValue({
      id: "22222222-2222-4222-8222-222222222222",
      organization_id: organizationId,
      stable_key: "cli-scout-workspace",
      display_name: "Scout workspace",
      coordinator_public_key: "public-key",
    });

    await expect(specialistCreateWorkspace(
      credentials,
      organizationId,
      "Scout workspace",
    )).resolves.toMatchObject({
      id: "22222222-2222-4222-8222-222222222222",
      organization_id: organizationId,
      display_name: "Scout workspace",
      status: "active",
      latest_change_sequence: 0,
      source_count: 0,
      active_machine_count: 0,
      run_count: 0,
      simulation_count: 0,
    });
    expect(product.request).toHaveBeenCalledWith("specialist.create_workspace", {
      organizationId,
      displayName: "Scout workspace",
    });
  });

  it("rejects a create receipt bound to another organization", async () => {
    product.request.mockResolvedValue({
      id: "22222222-2222-4222-8222-222222222222",
      organization_id: "33333333-3333-4333-8333-333333333333",
      stable_key: "cli-scout-workspace",
      display_name: "Scout workspace",
      coordinator_public_key: "public-key",
    });

    await expect(specialistCreateWorkspace(
      credentials,
      organizationId,
      "Scout workspace",
    )).rejects.toThrow("invalid Scout workspace");
  });
});
