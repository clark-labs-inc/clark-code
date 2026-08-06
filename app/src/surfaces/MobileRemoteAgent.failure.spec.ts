import { describe, expect, it } from "vitest";

import {
  MobileRemoteFailure,
  remoteFailureReceipt,
} from "./MobileRemoteAgent";

describe("remoteFailureReceipt", () => {
  it("preserves typed retryability without parsing error prose", () => {
    expect(remoteFailureReceipt(new MobileRemoteFailure(
      "desktop_unavailable",
      "temporary disconnect",
      true,
    ))).toEqual({
      error: "temporary disconnect",
      error_code: "desktop_unavailable",
      retryable: true,
    });
  });

  it("bounds untyped failures behind a stable fallback code", () => {
    expect(remoteFailureReceipt(new Error("provider broke"))).toEqual({
      error: "Error: provider broke",
      error_code: "command_failed",
      retryable: false,
    });
  });
});
