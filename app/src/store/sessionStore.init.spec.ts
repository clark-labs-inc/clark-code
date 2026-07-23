import { afterEach, describe, expect, it, vi } from "vitest";
import { MockBridge } from "../core-bridge/mockBridge";
import { useSessionStore } from "./sessionStore";

afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("session store initialization", () => {
  it("coalesces Strict Mode's concurrent init calls into one subscription", async () => {
    vi.useFakeTimers();
    const subscribe = vi.spyOn(MockBridge.prototype, "subscribe");

    await Promise.all([
      useSessionStore.getState().init(),
      useSessionStore.getState().init(),
    ]);

    expect(subscribe).toHaveBeenCalledOnce();
  });
});
