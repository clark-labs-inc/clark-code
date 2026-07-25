import { describe, expect, it } from "vitest";
import type { CodeRemoteCommand } from "./mobileRemote";
import { mobileRemoteModelSettings } from "./mobileRemoteModelSettings";

function command(payload?: Record<string, unknown>): CodeRemoteCommand {
  return {
    command_id: "command-1",
    host_id: "host-1",
    project_id: "project-1",
    desktop_id: "conversation-1",
    command_type: "send_message",
    request: { text: "Continue", ...(payload ? { payload } : {}) },
    status: "pending",
    created_at: "2026-07-22T12:00:00.000Z",
    updated_at: "2026-07-22T12:00:00.000Z",
  };
}

describe("mobileRemoteModelSettings", () => {
  it("preserves existing conversation settings when mobile sends no selection", () => {
    expect(mobileRemoteModelSettings(command())).toBeNull();
  });

  it("accepts supported model-specific effort pairs", () => {
    expect(mobileRemoteModelSettings(command({
      model: "clark-code",
      reasoning_effort: "xhigh",
    }))).toEqual({
      model: "clark-code",
      reasoningEffort: "xhigh",
    });
    expect(mobileRemoteModelSettings(command({
      model: "clark-code:grok45",
      reasoning_effort: "low",
    }))).toEqual({
      model: "clark-code:grok45",
      reasoningEffort: "low",
    });
    expect(mobileRemoteModelSettings(command({
      model: "clark-code:claude_opus_5",
      reasoning_effort: "",
    }))).toEqual({
      model: "clark-code:claude_opus_5",
      reasoningEffort: "",
    });
    expect(mobileRemoteModelSettings(command({
      model: "clark-code:gpt56_sol",
      reasoning_effort: "",
    }))).toEqual({
      model: "clark-code:gpt56_sol",
      reasoningEffort: "",
    });
  });

  it("rejects stale model ids and unsupported effort choices", () => {
    expect(() => mobileRemoteModelSettings(command({
      model: "clark-code:retired",
      reasoning_effort: "",
    }))).toThrow("not available");
    expect(() => mobileRemoteModelSettings(command({
      model: "clark-code:kimi_k3",
      reasoning_effort: "low",
    }))).toThrow("reasoning effort");
  });
});
