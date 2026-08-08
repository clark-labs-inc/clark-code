import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

import {
  CODE_REMOTE_CAPABILITIES,
  CODE_REMOTE_PROTOCOL_VERSION,
  registerCodeRemoteHost,
} from "./mobileRemote";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

describe("registerCodeRemoteHost", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockResolvedValue({});
  });

  it("publishes a real version and the complete protocol-v2 capability set", async () => {
    await registerCodeRemoteHost(
      { accountScope: "id:account-one" },
      {
        hostId: "host-1",
        displayName: "Stan desktop",
        os: "MacIntel",
        arch: "arm64",
        appVersion: "0.1.92",
        protocolVersion: CODE_REMOTE_PROTOCOL_VERSION,
        capabilities: CODE_REMOTE_CAPABILITIES,
        projects: [],
      },
    );

    expect(invoke).toHaveBeenCalledWith("product_request", {
      operation: "mobile.host_upsert",
      payload: {
        hostId: "host-1",
        displayName: "Stan desktop",
        osName: "MacIntel",
        arch: "arm64",
        appVersion: "0.1.92",
        protocolVersion: 2,
        capabilities: [
          "start_session",
          "send_message",
          "cancel_run",
          "resolve_permission",
          "steer_run",
          "compact_conversation",
          "edit_and_resend",
        ],
        projects: [],
      },
    });
  });
});
