import { useEffect, useRef } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { useSessionStore } from "../store/sessionStore";
import { isBusy, liveSessions, mergedOf } from "../store/sessionStore.runtime";
import type { PromptReceipt } from "../core-bridge/bridge";
import { cloudCreds, type CloudCreds } from "../lib/cloudHistory";
import {
  CODE_REMOTE_CAPABILITIES,
  CODE_REMOTE_PROTOCOL_VERSION,
  ackCodeRemoteCommand,
  downloadCodeRemoteAttachment,
  pollCodeRemoteCommands,
  registerCodeRemoteHost,
  type CodeRemoteCommand,
  type CodeRemoteProjectRegistration,
} from "../lib/mobileRemote";
import type { PendingAttachment } from "../lib/attachments";
import { loadSshHosts, hostLabel, hostReady } from "../lib/sshHosts";
import { pickAllowOption } from "../lib/permissions";
import { notify } from "../lib/notify";
import { isAuthExpiredError, refreshAuthSession } from "../lib/auth";
import {
  discoverRepositories,
  projectKnowledgeEnabled,
  repositoryIdentityForRoot,
  repositoriesUnderRoot,
  syncRepositoriesUnderRoot,
} from "../lib/repositoryKnowledge";
import { desktopHostId, desktopInstanceId } from "../lib/desktopHost";
import { mobileRemoteRetryDelayMs } from "../lib/mobileRemoteRetry";
import {
  mobileRemoteModelSettings,
  type MobileRemoteModelSettings,
} from "../lib/mobileRemoteModelSettings";
import {
  ensureMobileRemoteLiveTarget,
  inspectMobileRemoteTarget,
} from "../lib/mobileRemoteLiveTarget";
import {
  MobileRemotePresenceLoop,
  publishMobileRemotePresence,
} from "../lib/mobileRemotePresence";
import { mobileRemoteCommandWaitsForIdle } from "../lib/mobileRemoteCommandScheduling";

const LOOP_INTERVAL_MS = 500;
const COMMAND_POLL_WAIT_MS = 25_000;

type MobileRemoteFailureCode =
  | "invalid_command"
  | "invalid_claim"
  | "desktop_unavailable"
  | "project_unavailable"
  | "conversation_busy"
  | "stale_run"
  | "stale_edit"
  | "stale_permission"
  | "submission_failed"
  | "command_failed";

export class MobileRemoteFailure extends Error {
  constructor(
    readonly code: MobileRemoteFailureCode,
    message: string,
    readonly retryable = false,
  ) {
    super(message);
  }
}

function remoteFailure(
  code: MobileRemoteFailureCode,
  message: string,
  retryable = false,
): never {
  throw new MobileRemoteFailure(code, message, retryable);
}

export function remoteFailureReceipt(error: unknown): {
  error: string;
  error_code: MobileRemoteFailureCode;
  retryable: boolean;
} {
  if (error instanceof MobileRemoteFailure) {
    return {
      error: error.message.slice(0, 800),
      error_code: error.code,
      retryable: error.retryable,
    };
  }
  return {
    error: String(error).slice(0, 800),
    error_code: "command_failed",
    retryable: false,
  };
}

function leaf(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() || path || "Project";
}

function localProjectId(root: string): string {
  return `local:${encodeURIComponent(root)}`;
}

function sshProjectId(hostId: string, root: string): string {
  return `ssh:${encodeURIComponent(hostId)}:${encodeURIComponent(root)}`;
}

function currentProjects(): CodeRemoteProjectRegistration[] {
  const state = useSessionStore.getState();
  const projects: CodeRemoteProjectRegistration[] = [];
  const cwd = state.localSettings.cwd.trim();
  if (cwd) {
    const repositories = repositoriesUnderRoot(cwd);
    if (repositories.length > 0) {
      for (const repository of repositories) {
        projects.push({
          id: localProjectId(repository.root),
          kind: "local",
          display_name: leaf(repository.root),
          root: repository.root,
          trusted: true,
          repository_fingerprint: repository.fingerprint,
        });
      }
    } else {
      const repository = repositoryIdentityForRoot(cwd);
      projects.push({
        id: localProjectId(cwd),
        kind: "local",
        display_name: leaf(cwd),
        root: cwd,
        trusted: true,
        repository_fingerprint: repository?.fingerprint ?? null,
      });
    }
  }
  for (const host of loadSshHosts().filter(hostReady)) {
    projects.push({
      id: sshProjectId(host.id, host.remoteRoot.trim()),
      kind: "ssh",
      display_name: hostLabel(host),
      root: host.remoteRoot.trim(),
      ssh_alias: host.host.trim(),
      trusted: true,
    });
  }
  return projects;
}

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
    const receipt = await useSessionStore.getState().send(text);
    if (!receipt) {
      remoteFailure("submission_failed", "Clark Code did not start the mobile prompt.");
    }
    return receipt;
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
  const receipt = await submission;
  if (!receipt) {
    remoteFailure("submission_failed", "Clark Code did not start the mobile prompt.");
  }
  return receipt;
}

function commandBool(command: CodeRemoteCommand, key: string): boolean | null {
  const value = command.request[key];
  return typeof value === "boolean" ? value : null;
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

function commandWaitsForTargetIdle(command: CodeRemoteCommand): boolean {
  const entry = command.desktop_id ? liveSessions.get(command.desktop_id) : null;
  return mobileRemoteCommandWaitsForIdle(
    command,
    entry ? isBusy(mergedOf(entry)) : false,
  );
}

function projectFor(command: CodeRemoteCommand): CodeRemoteProjectRegistration | null {
  if (!command.project_id) return null;
  return currentProjects().find((project) => project.id === command.project_id) ?? null;
}

async function applyProject(command: CodeRemoteCommand): Promise<void> {
  const project = projectFor(command);
  if (!project) {
    remoteFailure("project_unavailable", "Project is not available on this desktop host.");
  }
  const state = useSessionStore.getState();
  state.selectProvider("local");
  if (project.kind === "local") {
    state.setProjectMode("local");
    state.setProjectFolder(project.root);
    return;
  }
  const host = loadSshHosts().find((item) => sshProjectId(item.id, item.remoteRoot.trim()) === project.id);
  if (!host) {
    remoteFailure(
      "project_unavailable",
      "SSH project is no longer configured on this desktop host.",
    );
  }
  state.setProjectMode("remote");
  state.setSelectedHostId(host.id);
}

async function submitPrompt(
  creds: CloudCreds,
  command: CodeRemoteCommand,
): Promise<{
  modelSettings: MobileRemoteModelSettings | null;
  promptReceipt: PromptReceipt;
}> {
  const text = commandText(command);
  const attachments = await commandAttachments(creds, command);
  const modelSettings = mobileRemoteModelSettings(command);
  if (!text && attachments.length === 0) {
    remoteFailure("invalid_command", "Clark Code command has no prompt or attachments.");
  }
  await applyProject(command);
  const state = useSessionStore.getState();
  if (command.command_type === "start_session") {
    state.endSession();
    await useSessionStore.getState().startSession();
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
  const promptReceipt = await sendRemotePrompt(text, attachments);
  return { modelSettings, promptReceipt };
}

async function resolvePermission(command: CodeRemoteCommand): Promise<void> {
  const { state, entry, snapshot } = await requireLiveDesktop(command);
  const actionId = commandString(command, "action_id");
  const runId = commandRunId(command);
  const approved = commandBool(command, "approved");
  const pending = snapshot.pending_permission;
  const run = runId ? snapshot.runs[runId] : null;
  if (!actionId || !runId || approved === null) {
    remoteFailure("invalid_command", "Clark Code permission command is incomplete.");
  }
  if (!run || run.status !== "awaiting_input") {
    remoteFailure("stale_permission", "That run is not waiting for mobile approval.");
  }
  if (pending?.session !== command.desktop_id) {
    remoteFailure(
      "invalid_command",
      "That permission request belongs to another desktop conversation.",
    );
  }
  if (!pending || pending.id !== actionId) {
    remoteFailure("stale_permission", "That permission request is no longer pending.");
  }
  const option = approved
    ? pickAllowOption(pending)
    : pending.options.find((item) => item.kind === "reject_once" || item.kind === "reject_always");
  if (!option) {
    remoteFailure("invalid_command", "No matching permission option is available.");
  }
  if (!state.bridge) {
    remoteFailure("desktop_unavailable", "Clark Code desktop session is not connected.", true);
  }
  await state.bridge.respond(entry.session.id, {
    kind: "permission",
    request: pending.id,
    option: option.id,
  });
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
    remoteFailure("desktop_unavailable", "This Clark Code session cannot accept steering.", true);
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
    remoteFailure("desktop_unavailable", "This Clark Code session cannot compact context.", true);
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

async function runCommand(
  creds: CloudCreds,
  hostId: string,
  instanceId: string,
  command: CodeRemoteCommand,
  stillCurrent: () => boolean,
): Promise<void> {
  if (!command.claim_token) {
    remoteFailure("invalid_claim", "Clark Code command has no execution claim.");
  }
  const accepted = await ackCodeRemoteCommand(
    creds,
    hostId,
    instanceId,
    command.claim_token,
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
      runId = submitted.promptReceipt.runId;
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
    const sessionId = command.desktop_id ?? useSessionStore.getState().session?.id ?? null;
    const completed = await ackCodeRemoteCommand(
      creds,
      hostId,
      instanceId,
      command.claim_token,
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
        command.claim_token,
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

export function MobileRemoteAgent() {
  const auth = useSessionStore((state) => state.auth);
  const cwd = useSessionStore((state) => state.localSettings.cwd);
  const commandBusyRef = useRef(false);
  const repositoryBusyRef = useRef(false);
  const consecutiveFailuresRef = useRef(0);
  const retryAtRef = useRef(0);

  useEffect(() => {
    if (!auth) return;
    const hostId = desktopHostId();
    const instanceId = desktopInstanceId();
    let stopped = false;
    let authRefresh: Promise<void> | null = null;
    let appVersion: Promise<string> | null = null;
    consecutiveFailuresRef.current = 0;
    retryAtRef.current = 0;

    const recoverExpiredAuth = async (error: unknown): Promise<boolean> => {
      if (!isAuthExpiredError(error)) return false;
      if (!authRefresh) {
        const attempt = (async () => {
          const currentAuth = useSessionStore.getState().auth;
          const refreshed = currentAuth ? await refreshAuthSession(currentAuth) : null;
          if (stopped) return;
          if (refreshed) {
            useSessionStore.setState({ auth: refreshed });
          } else {
            useSessionStore.getState().signOutAuth();
            void notify("Clark sign-in expired", "Sign in again to keep Clark Code remote control online.");
          }
        })();
        authRefresh = attempt;
        void attempt.finally(() => {
          if (authRefresh === attempt) authRefresh = null;
        });
      }
      await authRefresh;
      return true;
    };

    const refreshPresence = async () => {
      if (stopped || navigator.onLine === false) return;
      const creds = cloudCreds(useSessionStore.getState().auth);
      if (!creds) return;
      const root = useSessionStore.getState().localSettings.cwd.trim();
      const refreshRepositories = root && projectKnowledgeEnabled() && !repositoryBusyRef.current
        ? async () => {
            repositoryBusyRef.current = true;
            try {
              await discoverRepositories(root);
              const currentCreds = cloudCreds(useSessionStore.getState().auth);
              if (currentCreds) await syncRepositoriesUnderRoot(currentCreds, root);
            } finally {
              repositoryBusyRef.current = false;
            }
          }
        : undefined;
      try {
        await publishMobileRemotePresence(
          async () => {
            appVersion ??= getVersion();
            await registerCodeRemoteHost(creds, {
              hostId,
              displayName: `${auth.user.name || "Clark"} desktop`,
              os: navigator.platform || "desktop",
              arch: "",
              appVersion: await appVersion,
              protocolVersion: CODE_REMOTE_PROTOCOL_VERSION,
              capabilities: CODE_REMOTE_CAPABILITIES,
              projects: currentProjects(),
            });
          },
          refreshRepositories,
        );
      } catch (error) {
        await recoverExpiredAuth(error);
      }
    };

    const presenceLoop = new MobileRemotePresenceLoop(refreshPresence);

    const pollCommands = async () => {
      if (stopped || commandBusyRef.current) return;
      if (navigator.onLine === false || Date.now() < retryAtRef.current) return;
      const creds = cloudCreds(useSessionStore.getState().auth);
      if (!creds) return;
      commandBusyRef.current = true;
      try {
        const response = await pollCodeRemoteCommands(
          creds,
          hostId,
          instanceId,
          1,
          COMMAND_POLL_WAIT_MS,
        );
        for (const command of response.commands) {
          if (stopped) break;
          // Leave a busy follow-up in `delivered`, where it survives a desktop
          // restart. Acknowledging it and handing it to `send()` would put it
          // in the process-local queue and falsely report completion to mobile.
          if (commandWaitsForTargetIdle(command)) continue;
          await runCommand(
            creds,
            hostId,
            instanceId,
            command,
            () => !stopped && cloudCreds(useSessionStore.getState().auth)?.token === creds.token,
          );
        }
        consecutiveFailuresRef.current = 0;
        retryAtRef.current = 0;
      } catch (error) {
        if (!(await recoverExpiredAuth(error))) {
          consecutiveFailuresRef.current += 1;
          retryAtRef.current = Date.now() + mobileRemoteRetryDelayMs(consecutiveFailuresRef.current);
        }
        /* Remote control is a background affordance; normal desktop use continues. */
      } finally {
        commandBusyRef.current = false;
      }
    };

    presenceLoop.start();
    void pollCommands();
    const timer = window.setInterval(() => void pollCommands(), LOOP_INTERVAL_MS);
    const resumeAfterOutage = () => {
      consecutiveFailuresRef.current = 0;
      retryAtRef.current = 0;
      presenceLoop.refreshNow();
      void pollCommands();
    };
    const resumeWhenVisible = () => {
      if (document.visibilityState === "visible") resumeAfterOutage();
    };
    window.addEventListener("online", resumeAfterOutage);
    window.addEventListener("focus", resumeAfterOutage);
    document.addEventListener("visibilitychange", resumeWhenVisible);
    return () => {
      stopped = true;
      presenceLoop.stop();
      window.clearInterval(timer);
      window.removeEventListener("online", resumeAfterOutage);
      window.removeEventListener("focus", resumeAfterOutage);
      document.removeEventListener("visibilitychange", resumeWhenVisible);
    };
  }, [auth, cwd]);

  return null;
}
