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

  it("always resolves selected models to their maximum effort", () => {
    expect(mobileRemoteModelSettings(command({
      model: "local-model-large",
      reasoning_effort: "xhigh",
    }))).toEqual({
      model: "local-model-large",
      reasoningEffort: "max",
    });
    expect(mobileRemoteModelSettings(command({
      model: "local-model",
      reasoning_effort: "low",
    }))).toEqual({
      model: "local-model",
      reasoningEffort: "high",
    });
  });

  it("rejects stale model ids but ignores old effort choices", () => {
    expect(() => mobileRemoteModelSettings(command({
      model: "retired-model",
      reasoning_effort: "",
    }))).toThrow("not available");
    expect(mobileRemoteModelSettings(command({
      model: "local-model-large",
      reasoning_effort: "low",
    }))).toEqual({ model: "local-model-large", reasoningEffort: "max" });
  });
});
