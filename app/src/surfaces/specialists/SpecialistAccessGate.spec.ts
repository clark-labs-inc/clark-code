import { describe, expect, it, vi } from "vitest";
import { runSpecialistGateAction } from "./SpecialistAccessGate";

describe("specialist access gate actions", () => {
  it("routes a product action to billing instead of retrying access", () => {
    const handlers = {
      signIn: vi.fn(),
      retry: vi.fn(),
      productAction: vi.fn(),
      setupWorkspace: vi.fn(),
    };

    runSpecialistGateAction("product_action", handlers);

    expect(handlers.productAction).toHaveBeenCalledOnce();
    expect(handlers.retry).not.toHaveBeenCalled();
    expect(handlers.signIn).not.toHaveBeenCalled();
    expect(handlers.setupWorkspace).not.toHaveBeenCalled();
  });

  it("routes workspace setup to the product's explicit Scout setup flow", () => {
    const handlers = {
      signIn: vi.fn(),
      retry: vi.fn(),
      productAction: vi.fn(),
      setupWorkspace: vi.fn(),
    };

    runSpecialistGateAction("setup_workspace", handlers);

    expect(handlers.setupWorkspace).toHaveBeenCalledOnce();
    expect(handlers.productAction).not.toHaveBeenCalled();
  });
});
