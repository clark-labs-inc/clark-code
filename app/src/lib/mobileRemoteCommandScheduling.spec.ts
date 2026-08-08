import { describe, expect, it } from "vitest";
import { mobileRemoteCommandWaitsForIdle } from "./mobileRemoteCommandScheduling";

describe("mobileRemoteCommandWaitsForIdle", () => {
  it("keeps a follow-up durable while its target conversation is busy", () => {
    expect(mobileRemoteCommandWaitsForIdle({
      command_type: "send_message",
      desktop_id: "conversation-1",
    }, true)).toBe(true);
  });

  it("releases the follow-up as soon as its target is idle", () => {
    expect(mobileRemoteCommandWaitsForIdle({
      command_type: "send_message",
      desktop_id: "conversation-1",
    }, false)).toBe(false);
  });

  it("keeps compact and edit commands durable until their target is idle", () => {
    for (const command_type of ["compact_conversation", "edit_and_resend"] as const) {
      expect(mobileRemoteCommandWaitsForIdle({
        command_type,
        desktop_id: "conversation-1",
      }, true)).toBe(true);
      expect(mobileRemoteCommandWaitsForIdle({
        command_type,
        desktop_id: "conversation-1",
      }, false)).toBe(false);
    }
  });

  it("does not delay cancellation, permission, or active-run steering", () => {
    expect(mobileRemoteCommandWaitsForIdle({
      command_type: "cancel_run",
      desktop_id: "conversation-1",
    }, true)).toBe(false);
    expect(mobileRemoteCommandWaitsForIdle({
      command_type: "resolve_permission",
      desktop_id: "conversation-1",
    }, true)).toBe(false);
    expect(mobileRemoteCommandWaitsForIdle({
      command_type: "steer_run",
      desktop_id: "conversation-1",
    }, true)).toBe(false);
  });
});
