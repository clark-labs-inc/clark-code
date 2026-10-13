import { describe, expect, it } from "vitest";
import { emptySnapshot, type Snapshot } from "../core-bridge/types";
import type { CodeRemoteCommand } from "./mobileRemote";
import { mobileRemotePermissionResponse } from "./mobileRemotePermission";

function snapshot(): Snapshot {
  return {
    ...emptySnapshot(),
    runs: { r1: { id: "r1", status: "running" }, r2: { id: "r2", status: "running" } },
    timeline: [{ item: "tool_call", id: "shell", run: "r2" }],
    pending_permission: {
      id: "approval", session: "chat", tool_call: "shell", title: "Run command?",
      options: [
        { id: "always", kind: "allow_always", label: "Always" },
        { id: "once", kind: "allow_once", label: "Once" },
        { id: "reject", kind: "reject_once", label: "Reject" },
      ],
    },
  };
}
function command(request: Record<string, unknown> = {}): CodeRemoteCommand {
  return {
    command_id: "command", host_id: "host", desktop_id: "chat",
    command_type: "resolve_permission", status: "delivered",
    created_at: "", updated_at: "",
    request: { action_id: "approval", run_id: "r2", approved: true, ...request },
  };
}
describe("mobile permission boundary", () => {
  it("accepts the exact tool owner while the projected run remains running", () => {
    expect(mobileRemotePermissionResponse(snapshot(), command())).toEqual({
      kind: "permission", request: "approval", option: "once",
    });
  });
  it("also supports providers that project awaiting_input", () => {
    const state = snapshot();
    state.runs.r2.status = "awaiting_input";
    expect(mobileRemotePermissionResponse(state, command()).kind).toBe("permission");
  });
  it.each(["done", "failed", "cancelled", "queued"] as const)("rejects a %s run", (status) => {
    const state = snapshot();
    state.runs.r2.status = status;
    expect(() => mobileRemotePermissionResponse(state, command())).toThrowError();
  });
  it("rejects a different active run, stale action, or another conversation", () => {
    expect(() => mobileRemotePermissionResponse(snapshot(), command({ run_id: "r1" }))).toThrowError();
    expect(() => mobileRemotePermissionResponse(snapshot(), command({ action_id: "older" }))).toThrowError();
    expect(() => mobileRemotePermissionResponse(snapshot(), { ...command(), desktop_id: "other" })).toThrowError();
  });
  it("accepts an unlinked legacy request only with one unambiguous active run", () => {
    const state = snapshot();
    state.timeline = [];
    expect(() => mobileRemotePermissionResponse(state, command())).toThrowError();
    state.runs.r1.status = "done";
    expect(mobileRemotePermissionResponse(state, command()).kind).toBe("permission");
  });
  it("fails closed when the decision or an offered option is missing", () => {
    expect(() => mobileRemotePermissionResponse(snapshot(), command({ approved: null }))).toThrowError();
    const state = snapshot();
    state.pending_permission!.options = [];
    expect(() => mobileRemotePermissionResponse(state, command())).toThrowError();
  });
});
