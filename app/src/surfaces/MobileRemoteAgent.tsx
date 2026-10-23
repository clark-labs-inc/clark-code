import { useSessionStore } from "../store/sessionStore";
import { isBusy, liveSessions, mergedOf } from "../store/sessionStore.runtime";
import type { PromptReceipt } from "../core-bridge/bridge";
import { type CloudCreds } from "../lib/cloudHistory";
import {
  ackCodeRemoteCommand,
  downloadCodeRemoteAttachment,
  type CodeRemoteCommand,
} from "../lib/mobileRemote";
import type { PendingAttachment } from "../lib/attachments";
import { mobileRemotePermissionResponse } from "../lib/mobileRemotePermission";
import { notify } from "../lib/notify";
import {
  mobileRemoteModelSettings,
  type MobileRemoteModelSettings,
} from "../lib/mobileRemoteModelSettings";
import {
  ensureMobileRemoteLiveTarget,
  inspectMobileRemoteTarget,
} from "../lib/mobileRemoteLiveTarget";
import { mobileRemoteCommandWaitsForIdle } from "../lib/mobileRemoteCommandScheduling";


import { applyProject, startMobileRemoteSession } from "../lib/mobileRemoteProjects";
export { currentProjects } from "../lib/mobileRemoteProjects";

import { remoteFailure, remoteFailureReceipt } from "../lib/mobileRemoteFailure";

function commandText(command: CodeRemoteCommand): string {
  const value = command.request.text;
  return typeof value === "string" ? value.trim() : "";
}

function commandAttachmentIds(command: CodeRemoteCommand): string[] {
  const value = command.request.attachments;
  if (!Array.isArray(value)) return [];
  return value.flatMap((item) => {
    if (!item || typeof item !== "object") return [];
    const attachmentId = (item as Record<string, unknown>).attachment_id;
    return typeof attachmentId === "string" && attachmentId.trim()
      ? [attachmentId.trim()]
      : [];
  });
}

async function commandAttachments(
  creds: CloudCreds,
  command: CodeRemoteCommand,
): Promise<PendingAttachment[]> {
  return Promise.all(commandAttachmentIds(command).map(async (attachmentId) => {
    const downloaded = await downloadCodeRemoteAttachment(
      creds,
      command.command_id,
      attachmentId,
    );
    return {
      id: `remote-${attachmentId}`,
      filename: downloaded.filename,
      content_type: downloaded.content_type,
      data_base64: downloaded.data_base64,
      size: downloaded.size_bytes,
    };
  }));
}

async function sendRemotePrompt(
  text: string,
  attachments: PendingAttachment[],
): Promise<PromptReceipt> {
  if (attachments.length === 0) {
    const outcome = await useSessionStore.getState().send(text);
    if (outcome.kind !== "started") {
      remoteFailure("submission_failed", "Clark Code did not start the mobile prompt.");
    }
    return outcome.receipt;
  }
  // `send` owns the queue/busy semantics but normally consumes the visible
  // composer attachments. Stage only the remote batch for that synchronous
  // handoff, then immediately restore anything the desktop user was composing.
  const preserved = useSessionStore.getState().attachments;
  useSessionStore.setState({ attachments });
  const submission = useSessionStore.getState().send(text);
  useSessionStore.setState((current) => ({
    attachments: [...preserved, ...current.attachments],
  }));
  const outcome = await submission;
  if (outcome.kind !== "started") {
    remoteFailure("submission_failed", "Clark Code did not start the mobile prompt.");
  }
  return outcome.receipt;
}

function commandString(command: CodeRemoteCommand, key: string): string | null {
  const value = command.request[key];
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function commandRunId(command: CodeRemoteCommand): string | null {
  return commandString(command, "run_id");
}

function commandPayload(command: CodeRemoteCommand): Record<string, unknown> {
  const payload = command.request.payload;
  return payload && typeof payload === "object" && !Array.isArray(payload)
    ? payload as Record<string, unknown>
    : {};
}

function messageTextAt(snapshot: ReturnType<typeof mergedOf>, timelineIndex: number): string | null {
  const item = snapshot.timeline[timelineIndex];
  if (item?.item !== "message" || item.role !== "user") return null;
  return item.blocks
    .filter((block) => block.type === "text")
    .map((block) => block.text)
    .join("\n");
}

async function requireLiveDesktop(command: CodeRemoteCommand) {
  if (!command.desktop_id) {
    remoteFailure("invalid_command", "Clark Code command is not bound to a desktop conversation.");
  }
  const state = useSessionStore.getState();
  const entry = await ensureMobileRemoteLiveTarget(command.desktop_id);
  if (!entry) {
    remoteFailure(
      "desktop_unavailable",
      "Clark Code is not connected to that desktop conversation.",
      true,
    );
  }
  return { state, entry, snapshot: mergedOf(entry) };
}

async function requireActiveRunDesktop(command: CodeRemoteCommand) {
  if (!command.desktop_id) {
    remoteFailure("invalid_command", "Clark Code command is not bound to a desktop conversation.");
  }
  const runId = commandRunId(command);
  if (!runId) {
    remoteFailure("invalid_command", "Clark Code command is not bound to a run.");
  }
  const inspected = await inspectMobileRemoteTarget(command.desktop_id);
  const inspectedRun = inspected.snapshot?.runs[runId];
  if (
    inspected.snapshot
    && (
      !inspectedRun
      || !["running", "queued", "awaiting_input"].includes(inspectedRun.status)
    )
  ) {
    remoteFailure("stale_run", "That run is no longer active.");
  }
  const target = inspected.entry
    ? {
        state: useSessionStore.getState(),
        entry: inspected.entry,
        snapshot: inspected.snapshot!,
      }
    : await requireLiveDesktop(command);
  const run = target.snapshot.runs[runId];
  if (
    !run
    || !["running", "queued", "awaiting_input"].includes(run.status)
  ) {
    remoteFailure("stale_run", "That run is no longer active.");
  }
  return { ...target, runId };
}

export function commandWaitsForTargetIdle(command: CodeRemoteCommand): boolean {
  const entry = command.desktop_id ? liveSessions.get(command.desktop_id) : null;
  return mobileRemoteCommandWaitsForIdle(
    command,
    entry ? isBusy(mergedOf(entry)) : false,
  );
}

async function submitPrompt(
  creds: CloudCreds,
  command: CodeRemoteCommand,
): Promise<{
  modelSettings: MobileRemoteModelSettings | null;
  promptReceipt: PromptReceipt | null;
}> {
  const text = commandText(command);
  const attachments = await commandAttachments(creds, command);
  const modelSettings = mobileRemoteModelSettings(command);
  if (command.command_type !== "start_session" && !text && attachments.length === 0) {
    remoteFailure("invalid_command", "Clark Code command has no prompt or attachments.");
  }
  if (command.command_type !== "start_session") await applyProject(command);
  if (command.command_type === "start_session") {
    await startMobileRemoteSession(command);
  } else if (!command.desktop_id) {
    remoteFailure(
      "invalid_command",
      "Clark Code follow-up is not bound to a desktop conversation.",
    );
  } else if (command.desktop_id && useSessionStore.getState().session?.id !== command.desktop_id) {
    await useSessionStore.getState().openConversation(command.desktop_id);
  } else if (!useSessionStore.getState().session) {
    await useSessionStore.getState().startSession();
  }
  if (!useSessionStore.getState().session) {
    remoteFailure("desktop_unavailable", "Clark Code session did not start.", true);
  }
  if (
    command.command_type === "send_message" &&
    useSessionStore.getState().session?.id !== command.desktop_id
  ) {
    // Sessions run in parallel now, so an open should always bind; reaching
    // here means the open itself failed (e.g. its SSH host is gone).
    remoteFailure(
      "desktop_unavailable",
      "Could not open the target conversation on the desktop.",
      true,
    );
  }
  if (modelSettings) {
    await useSessionStore.getState().updateModelSettings(modelSettings);
  }
  const promptReceipt = text || attachments.length ? await sendRemotePrompt(text, attachments) : null;
  return { modelSettings, promptReceipt };
}

export async function resolvePermission(command: CodeRemoteCommand): Promise<void> {
  const { state, entry, snapshot } = await requireLiveDesktop(command);
  const response = mobileRemotePermissionResponse(snapshot, command);
  if (!state.bridge) {
    remoteFailure("desktop_unavailable", "Clark Code desktop session is not connected.", true);
  }
  await state.bridge.respond(entry.session.id, response);
}

export async function cancelRun(command: CodeRemoteCommand): Promise<void> {
  const { state, entry, runId } = await requireActiveRunDesktop(command);
  if (!state.bridge) {
    remoteFailure("desktop_unavailable", "Clark Code desktop session is not connected.", true);
  }
  await state.bridge.cancel(entry.session.id, runId);
}

export async function steerRun(command: CodeRemoteCommand): Promise<void> {
  const text = commandText(command);
  if (!text) {
    remoteFailure("invalid_command", "Clark Code steer command is incomplete.");
  }
  const { state, entry } = await requireActiveRunDesktop(command);
  if (!state.bridge?.steer) {
    remoteFailure("desktop_unavailable", "This desktop session cannot accept steering.", true);
  }
  await state.bridge.steer(entry.session.id, [{ type: "text", text }]);
}

export async function compactConversation(command: CodeRemoteCommand): Promise<void> {
  const { state, entry, snapshot } = await requireLiveDesktop(command);
  if (isBusy(snapshot)) {
    remoteFailure(
      "conversation_busy",
      "Clark Code is still working; compaction will retry when the run finishes.",
      true,
    );
  }
  if (!state.bridge?.compact) {
    remoteFailure("desktop_unavailable", "This desktop session cannot compact context.", true);
  }
  await state.bridge.compact(entry.session.id);
}

export async function editAndResend(command: CodeRemoteCommand): Promise<PromptReceipt> {
  const desktopId = command.desktop_id;
  if (!desktopId) {
    remoteFailure("invalid_command", "Clark Code edit command is not bound to a conversation.");
  }
  await applyProject(command);
  if (useSessionStore.getState().session?.id !== desktopId) {
    await useSessionStore.getState().openConversation(desktopId);
  }
  const { snapshot } = await requireLiveDesktop(command);
  if (isBusy(snapshot)) {
    remoteFailure(
      "conversation_busy",
      "Clark Code is still working; the edit will retry when the run finishes.",
      true,
    );
  }
  const payload = commandPayload(command);
  const timelineIndex = payload.timeline_index;
  const expectedText = payload.expected_text;
  if (
    typeof timelineIndex !== "number"
    || !Number.isSafeInteger(timelineIndex)
    || timelineIndex < 0
    || typeof expectedText !== "string"
  ) {
    remoteFailure("invalid_command", "Clark Code edit target is incomplete.");
  }
  if (messageTextAt(snapshot, timelineIndex) !== expectedText) {
    remoteFailure(
      "stale_edit",
      "That message changed on another device. Refresh the conversation before editing it.",
    );
  }
  const receipt = await useSessionStore.getState().resendFrom(
    timelineIndex,
    commandText(command),
  );
  if (!receipt) {
    remoteFailure("submission_failed", "Clark Code did not restart from the edited message.");
  }
  return receipt;
}

export async function runCommand(
  creds: CloudCreds,
  hostId: string,
  instanceId: string,
  command: CodeRemoteCommand,
  stillCurrent: () => boolean,
  publishProjects: () => Promise<void>,
): Promise<void> {
  const accepted = await ackCodeRemoteCommand(
    creds,
    hostId,
    instanceId,
    command.command_id,
    "accepted",
    {
      accepted_at: new Date().toISOString(),
      command_type: command.command_type,
    },
  );
  // A lease can expire while this desktop is recovering. Never execute an
  // action unless the service still granted this process the accepted claim.
  if (accepted.command.status !== "accepted" || !stillCurrent()) return;
  try {
    let modelSettings: MobileRemoteModelSettings | null = null;
    let runId: string | null = null;
    if (command.command_type === "start_session" || command.command_type === "send_message") {
      const submitted = await submitPrompt(creds, command);
      modelSettings = submitted.modelSettings;
      runId = submitted.promptReceipt?.runId ?? null;
      if (command.command_type === "start_session") await publishProjects();
    } else if (command.command_type === "cancel_run") {
      await cancelRun(command);
    } else if (command.command_type === "resolve_permission") {
      await resolvePermission(command);
    } else if (command.command_type === "steer_run") {
      await steerRun(command);
    } else if (command.command_type === "compact_conversation") {
      await compactConversation(command);
    } else if (command.command_type === "edit_and_resend") {
      const receipt = await editAndResend(command);
      runId = receipt.runId;
    } else {
      remoteFailure("invalid_command", `Unsupported Clark Code command: ${command.command_type}`);
    }
    if (!stillCurrent()) return;
    const sessionId = command.command_type === "start_session"
      ? useSessionStore.getState().session?.id ?? null
      : command.desktop_id ?? useSessionStore.getState().session?.id ?? null;
    const completed = await ackCodeRemoteCommand(
      creds,
      hostId,
      instanceId,
      command.command_id,
      "completed",
      {
        desktop_id: sessionId,
        ...(runId ? { run_id: runId } : {}),
        submitted_at: new Date().toISOString(),
        ...(modelSettings ? {
          model: modelSettings.model,
          reasoning_effort: modelSettings.reasoningEffort,
        } : {}),
      },
    );
    if (completed.command.status === "completed") {
      void notify("Clark Code", "Mobile command started on this desktop.");
    }
  } catch (error) {
    if (stillCurrent()) {
      const failure = remoteFailureReceipt(error);
      await ackCodeRemoteCommand(
        creds,
        hostId,
        instanceId,
        command.command_id,
        "failed",
        {
          ...failure,
          failed_at: new Date().toISOString(),
        },
      ).catch(() => undefined);
    }
  }
}

export { MobileRemoteAgent } from "./MobileRemoteConnection";
