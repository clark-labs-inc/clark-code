import { afterEach, describe, expect, it, vi } from "vitest";
import { withinOptionalContextBudget } from "./sessionStore.conversationActions";

afterEach(() => {
  vi.useRealTimers();
});

describe("optional specialist context startup budget", () => {
  it("does not strand session attachment on an unresolved cloud snapshot", async () => {
    vi.useFakeTimers();
    const pending = new Promise<string>(() => {});
    const bounded = withinOptionalContextBudget(pending, 25);
    const rejection = expect(bounded).rejects.toThrow("optional specialist context timed out");

    await vi.advanceTimersByTimeAsync(25);

    await rejection;
  });

  it("preserves context that arrives inside the startup budget", async () => {
    await expect(
      withinOptionalContextBudget(Promise.resolve("scout-context"), 25),
    ).resolves.toBe("scout-context");
  });
});
