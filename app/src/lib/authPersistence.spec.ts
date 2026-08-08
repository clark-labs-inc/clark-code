import { beforeEach, describe, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const descriptor = {
  user: { id: "account-a", name: "Account A", method: "google" as const },
};

describe("native auth persistence", () => {
  beforeEach(() => {
    vi.resetModules();
    invoke.mockReset();
    localStorage.clear();
    Object.defineProperty(globalThis, "window", {
      value: globalThis,
      configurable: true,
    });
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  });

  async function loadAuth() {
    const { installProductModule, neutralProduct } = await import("../product/productModule");
    installProductModule({
      branding: { id: "test-product", name: "Test Product", shortName: "Test" },
      authRequired: true,
      slots: {},
      localAgent: neutralProduct.localAgent,
      artifacts: neutralProduct.artifacts,
      errors: neutralProduct.errors,
    });
    return import("./auth");
  }

  it("does not request product authentication in the neutral build", async () => {
    const auth = await import("./auth");

    await auth.initializeAuthSession();

    expect(auth.loadAuthSession()).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("prefers the encrypted native session and clears it on sign-out", async () => {
    invoke.mockImplementation((command: string, args?: { operation?: string }) => {
      if (command === "product_request" && args?.operation === "account.load") {
        return Promise.resolve(descriptor);
      }
      return Promise.resolve();
    });
    const auth = await loadAuth();

    await auth.initializeAuthSession();
    await auth.signOut();

    expect(auth.loadAuthSession()).toBeNull();
    expect(invoke).toHaveBeenCalledWith("product_request", {
      operation: "account.sign_out",
      payload: {},
    });
  });

  it("keeps the in-memory account when native sign-out fails", async () => {
    invoke.mockImplementation((command: string, args?: { operation?: string }) => {
      if (command === "product_request" && args?.operation === "account.load") {
        return Promise.resolve(descriptor);
      }
      if (command === "product_request" && args?.operation === "account.sign_out") {
        return Promise.reject(new Error("disk unavailable"));
      }
      return Promise.resolve();
    });
    const auth = await loadAuth();
    await auth.initializeAuthSession();

    await expect(auth.signOut()).rejects.toThrow("disk unavailable");

    expect(auth.loadAuthSession()).toEqual(descriptor);
  });

  it("starts native sign-in without sending any OAuth or the agent credential", async () => {
    invoke.mockResolvedValue(descriptor);
    const auth = await loadAuth();

    await expect(auth.signInWithGoogle()).resolves.toEqual(descriptor);

    expect(invoke).toHaveBeenCalledWith("product_request", {
      operation: "account.sign_in",
      payload: {},
    });
    expect(JSON.stringify(invoke.mock.calls)).not.toMatch(/token|secret|clientId|clientSecret/i);
  });
});
