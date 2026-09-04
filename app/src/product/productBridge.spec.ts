import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import {
  configureProductAuthRecovery,
  productRequest,
  recoverProductAuthentication,
} from "./productBridge";

describe("product bridge", () => {
  beforeEach(() => {
    invoke.mockReset();
    configureProductAuthRecovery(null);
  });

  afterEach(() => {
    configureProductAuthRecovery(null);
  });

  it("rejects unsafe operations before native IPC", async () => {
    await expect(productRequest("../private-operation")).rejects.toThrow("invalid");
    await expect(productRequest("Uppercase")).rejects.toThrow("invalid");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("refreshes and replays the exact request once after an expired credential", async () => {
    const payload = { organizationId: "org-1" };
    const recover = vi.fn(async () => {});
    invoke
      .mockRejectedValueOnce(new Error("Clark access request failed: Clark session expired (401)"))
      .mockResolvedValueOnce({ allowed: true });
    configureProductAuthRecovery(recover);

    await expect(productRequest("specialist.entitlement", payload)).resolves.toEqual({
      allowed: true,
    });

    expect(recover).toHaveBeenCalledOnce();
    expect(invoke).toHaveBeenNthCalledWith(1, "product_request", {
      operation: "specialist.entitlement",
      payload,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "product_request", {
      operation: "specialist.entitlement",
      payload,
    });
  });

  it("coalesces concurrent expired requests onto one refresh", async () => {
    let releaseRecovery = () => {};
    let recovered = false;
    const recover = vi.fn(() => new Promise<void>((resolve) => {
      releaseRecovery = () => {
        recovered = true;
        resolve();
      };
    }));
    const operationCalls = new Map<string, number>();
    invoke.mockImplementation((_: string, args: { operation: string }) => {
      const count = (operationCalls.get(args.operation) ?? 0) + 1;
      operationCalls.set(args.operation, count);
      if (!recovered) return Promise.reject(new Error("Unauthorized: 401"));
      return Promise.resolve(args.operation);
    });
    configureProductAuthRecovery(recover);

    const access = productRequest("access.snapshot");
    const organizations = productRequest("specialist.organizations");
    await vi.waitFor(() => expect(recover).toHaveBeenCalledOnce());
    expect(invoke).toHaveBeenCalledTimes(2);

    releaseRecovery();
    await expect(Promise.all([access, organizations])).resolves.toEqual([
      "access.snapshot",
      "specialist.organizations",
    ]);
    expect(recover).toHaveBeenCalledOnce();
    expect(operationCalls).toEqual(new Map([
      ["access.snapshot", 2],
      ["specialist.organizations", 2],
    ]));
  });

  it("makes new requests wait for a native-event refresh already in flight", async () => {
    let releaseRecovery = () => {};
    const recover = vi.fn(() => new Promise<void>((resolve) => {
      releaseRecovery = resolve;
    }));
    configureProductAuthRecovery(recover);

    const nativeRecovery = recoverProductAuthentication();
    const request = productRequest("access.snapshot");
    await vi.waitFor(() => expect(recover).toHaveBeenCalledOnce());
    expect(invoke).not.toHaveBeenCalled();

    releaseRecovery();
    await nativeRecovery;
    await request;
    expect(invoke).toHaveBeenCalledOnce();
  });

  it("never loops when the replay is also unauthorized", async () => {
    const recover = vi.fn(async () => {});
    invoke.mockRejectedValue(new Error("401 Unauthorized"));
    configureProductAuthRecovery(recover);

    await expect(productRequest("access.snapshot")).rejects.toThrow("Unauthorized");
    expect(recover).toHaveBeenCalledOnce();
    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it("does not replay after a failed refresh", async () => {
    const recover = vi.fn(async () => {
      throw new Error("refresh rejected");
    });
    invoke.mockRejectedValue(new Error("401 Unauthorized"));
    configureProductAuthRecovery(recover);

    await expect(productRequest("access.snapshot")).rejects.toThrow("refresh rejected");
    expect(invoke).toHaveBeenCalledOnce();
  });

  it("does not recurse through account refresh or retry unrelated failures", async () => {
    const recover = vi.fn(async () => {});
    invoke.mockRejectedValueOnce(new Error("401 Unauthorized"));
    configureProductAuthRecovery(recover);

    await expect(productRequest("account.refresh")).rejects.toThrow("Unauthorized");
    expect(recover).not.toHaveBeenCalled();

    invoke.mockRejectedValueOnce(new Error("503 service unavailable"));
    await expect(productRequest("access.snapshot")).rejects.toThrow("503");
    expect(recover).not.toHaveBeenCalled();
  });
});
