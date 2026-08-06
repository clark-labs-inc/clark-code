import { afterEach, describe, expect, it, vi } from "vitest";

import { useSessionStore } from "../store/sessionStore";
import { liveSessions } from "../store/sessionStore.runtime";
import { ensureMobileRemoteLiveTarget } from "./mobileRemoteLiveTarget";

const originalOpenConversation = useSessionStore.getState().openConversation;

afterEach(() => {
  vi.useRealTimers();
  liveSessions.clear();
  useSessionStore.setState({
    opening: null,
    openConversation: originalOpenConversation,
  });
});

describe("mobile remote cold target", () => {
  it("settles when a backgrounded conversation open never resolves", async () => {
    vi.useFakeTimers();
    const openConversation = vi.fn(() => new Promise<void>(() => {}));
    useSessionStore.setState({ openConversation });

    const result = ensureMobileRemoteLiveTarget("cold-conversation");
    await vi.advanceTimersByTimeAsync(30_000);

    await expect(result).resolves.toBeNull();
    expect(openConversation).toHaveBeenCalledWith("cold-conversation");
  });
});
