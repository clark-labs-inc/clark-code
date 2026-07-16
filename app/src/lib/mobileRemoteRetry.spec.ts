import { describe, expect, it } from "vitest";
import {
  MOBILE_REMOTE_RETRY_MAX_MS,
  mobileRemoteRetryDelayMs,
} from "./mobileRemoteRetry";

describe("mobileRemoteRetryDelayMs", () => {
  it("backs off quickly enough to recover inside the mobile presence window", () => {
    expect([1, 2, 3, 4, 5, 6].map(mobileRemoteRetryDelayMs)).toEqual([
      1_000,
      2_000,
      4_000,
      8_000,
      16_000,
      30_000,
    ]);
  });

  it("caps long outages at thirty seconds", () => {
    expect(mobileRemoteRetryDelayMs(20)).toBe(MOBILE_REMOTE_RETRY_MAX_MS);
  });
});
