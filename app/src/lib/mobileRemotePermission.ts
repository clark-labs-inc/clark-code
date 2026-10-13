import type { ClientResponse, Snapshot } from "../core-bridge/types";
import type { CodeRemoteCommand } from "./mobileRemote";
import { remoteFailure } from "./mobileRemoteFailure";
import { pickAllowOption } from "./permissions";

/** PermissionRequest does not change the projection's run status. Validate
 * the live request and its owning run rather than requiring awaiting_input. */
export function mobileRemotePermissionResponse(
  snapshot: Snapshot,
  command: CodeRemoteCommand,
): ClientResponse {
  const { action_id: actionId, run_id: runId, approved } = command.request;
  if (typeof actionId !== "string" || !actionId.trim()
    || typeof runId !== "string" || !runId.trim() || typeof approved !== "boolean") {
    remoteFailure("invalid_command", "Clark Code permission command is incomplete.");
  }
  const pending = snapshot.pending_permission;
  if (!pending || pending.id !== actionId) {
    remoteFailure("stale_permission", "That permission request is no longer pending.");
  }
  if (pending.session !== command.desktop_id) {
    remoteFailure("invalid_command", "That permission request belongs to another desktop conversation.");
  }
  const run = snapshot.runs[runId];
  if (!run || (run.status !== "running" && run.status !== "awaiting_input")) {
    remoteFailure("stale_permission", "That run is no longer active.");
  }
  const owner = pending.tool_call ? snapshot.timeline.find((item) => (
    item.item === "tool_call" && item.id === pending.tool_call
  )) : undefined;
  const ownerRun = owner?.item === "tool_call" ? owner.run : undefined;
  if (ownerRun ? ownerRun !== runId : Object.values(snapshot.runs).filter((item) => (
    item.status === "running" || item.status === "awaiting_input"
  )).length !== 1) {
    remoteFailure("stale_permission", "That permission request belongs to another run.");
  }
  const option = approved ? pickAllowOption(pending)
    : pending.options.find((item) => item.kind === "reject_once")
      ?? pending.options.find((item) => item.kind === "reject_always");
  if (!option) {
    remoteFailure("invalid_command", "No matching permission option is available.");
  }
  return { kind: "permission", request: pending.id, option: option.id };
}
