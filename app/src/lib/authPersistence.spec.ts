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

  it("prefers the encrypted native session and clears it on sign-out", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "clark_account_load") {
        return Promise.resolve(descriptor);
      }
      return Promise.resolve();
    });
    const auth = await import("./auth");

    await auth.initializeAuthSession();
    await auth.signOut();

    expect(auth.loadAuthSession()).toBeNull();
    expect(invoke).toHaveBeenCalledWith("clark_sign_out");
  });

  it("keeps the in-memory account when native sign-out fails", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "clark_account_load") {
        return Promise.resolve(descriptor);
      }
      if (command === "clark_sign_out") return Promise.reject(new Error("disk unavailable"));
      return Promise.resolve();
    });
    const auth = await import("./auth");
    await auth.initializeAuthSession();

    await expect(auth.signOut()).rejects.toThrow("disk unavailable");

    expect(auth.loadAuthSession()).toEqual(descriptor);
  });

  it("starts native sign-in without sending any OAuth or Clark credential", async () => {
    invoke.mockResolvedValue(descriptor);
    const auth = await import("./auth");

    await expect(auth.signInWithGoogle()).resolves.toEqual(descriptor);

    expect(invoke).toHaveBeenCalledWith("clark_google_sign_in");
    expect(JSON.stringify(invoke.mock.calls)).not.toMatch(/token|secret|clientId|clientSecret/i);
  });
});
