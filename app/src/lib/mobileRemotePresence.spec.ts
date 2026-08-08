import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  MOBILE_REMOTE_HEARTBEAT_INTERVAL_MS,
  MobileRemotePresenceLoop,
  publishMobileRemotePresence,
} from "./mobileRemotePresence";

describe("publishMobileRemotePresence", () => {
  it("does not let slow repository discovery delay a published lease", async () => {
    let finishDiscovery: () => void = () => undefined;
    const publish = vi.fn(async () => undefined);
    const discover = vi.fn(() => new Promise<void>((resolve) => {
      finishDiscovery = resolve;
    }));

    await publishMobileRemotePresence(publish, discover);
    await Promise.resolve();

    expect(publish).toHaveBeenCalledTimes(1);
    expect(discover).toHaveBeenCalledTimes(1);
    finishDiscovery();
  });
});

describe("MobileRemotePresenceLoop", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("refreshes immediately and on a clock independent of other work", async () => {
    const refresh = vi.fn(async () => undefined);
    const loop = new MobileRemotePresenceLoop(refresh);

    loop.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(refresh).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(MOBILE_REMOTE_HEARTBEAT_INTERVAL_MS * 3);
    expect(refresh).toHaveBeenCalledTimes(4);
    loop.stop();
  });

  it("coalesces timer ticks while a presence request is in flight", async () => {
    let finishRefresh: () => void = () => undefined;
    const refresh = vi.fn(() => new Promise<void>((resolve) => {
      finishRefresh = resolve;
    }));
    const loop = new MobileRemotePresenceLoop(refresh);

    loop.start();
    await vi.advanceTimersByTimeAsync(MOBILE_REMOTE_HEARTBEAT_INTERVAL_MS * 3);
    expect(refresh).toHaveBeenCalledTimes(1);

    finishRefresh();
    await vi.advanceTimersByTimeAsync(0);
    expect(refresh).toHaveBeenCalledTimes(2);
    loop.stop();
  });

  it("republishes immediately after wake even when the pre-sleep request was frozen", async () => {
    let finishRefresh: () => void = () => undefined;
    const refresh = vi.fn(() => new Promise<void>((resolve) => {
      finishRefresh = resolve;
    }));
    const loop = new MobileRemotePresenceLoop(refresh);

    loop.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(refresh).toHaveBeenCalledTimes(1);

    // A macOS wake/focus signal arrives while the old network future is still
    // frozen. It must become an immediate pending refresh, not wait 30 seconds.
    loop.refreshNow();
    finishRefresh();
    await vi.advanceTimersByTimeAsync(0);
    expect(refresh).toHaveBeenCalledTimes(2);
    loop.stop();
  });

  it("stops scheduled refreshes", async () => {
    const refresh = vi.fn(async () => undefined);
    const loop = new MobileRemotePresenceLoop(refresh);

    loop.start();
    await vi.advanceTimersByTimeAsync(0);
    loop.stop();
    await vi.advanceTimersByTimeAsync(MOBILE_REMOTE_HEARTBEAT_INTERVAL_MS * 2);

    expect(refresh).toHaveBeenCalledTimes(1);
  });
});
