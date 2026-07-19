import { useEffect, useRef } from "react";
import { useSessionStore } from "../store/sessionStore";
import { cloudCreds, type CloudCreds } from "../lib/cloudHistory";
import {
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
import { desktopHostId } from "../lib/desktopHost";
import { mobileRemoteRetryDelayMs } from "../lib/mobileRemoteRetry";
import {
  MobileRemotePresenceLoop,
  publishMobileRemotePresence,
} from "../lib/mobileRemotePresence";

const LOOP_INTERVAL_MS = 500;
const COMMAND_POLL_WAIT_MS = 25_000;

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

async function sendRemotePrompt(text: string, attachments: PendingAttachment[]): Promise<void> {
  if (attachments.length === 0) {
    await useSessionStore.getState().send(text);
    return;
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
  await submission;
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

function requireActiveDesktop(command: CodeRemoteCommand) {
  if (!command.desktop_id) {
    throw new Error("Clark Code command is not bound to a desktop conversation.");
  }
  const state = useSessionStore.getState();
  if (!state.session || state.session.id !== command.desktop_id) {
    throw new Error("Clark Code command is not for the active desktop conversation.");
  }
  return state;
}

function projectFor(command: CodeRemoteCommand): CodeRemoteProjectRegistration | null {
  if (!command.project_id) return null;
  return currentProjects().find((project) => project.id === command.project_id) ?? null;
}

async function applyProject(command: CodeRemoteCommand): Promise<void> {
  const project = projectFor(command);
  if (!project) {
    throw new Error("Project is not available on this desktop host.");
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
    throw new Error("SSH project is no longer configured on this desktop host.");
  }
  state.setProjectMode("remote");
  state.setSelectedHostId(host.id);
}

async function submitPrompt(creds: CloudCreds, command: CodeRemoteCommand): Promise<void> {
  const text = commandText(command);
  const attachments = await commandAttachments(creds, command);
  if (!text && attachments.length === 0) {
    throw new Error("Clark Code command has no prompt or attachments.");
  }
  await applyProject(command);
  const state = useSessionStore.getState();
  if (command.command_type === "start_session") {
    state.endSession();
    await useSessionStore.getState().startSession();
  } else if (!command.desktop_id) {
    throw new Error("Clark Code follow-up is not bound to a desktop conversation.");
  } else if (command.desktop_id && useSessionStore.getState().session?.id !== command.desktop_id) {
    await useSessionStore.getState().openConversation(command.desktop_id);
  } else if (!useSessionStore.getState().session) {
    await useSessionStore.getState().startSession();
  }
  if (!useSessionStore.getState().session) {
    throw new Error("Clark Code session did not start.");
  }
  if (
    command.command_type === "send_message" &&
    useSessionStore.getState().session?.id !== command.desktop_id
  ) {
    // Sessions run in parallel now, so an open should always bind; reaching
    // here means the open itself failed (e.g. its SSH host is gone).
    throw new Error("Could not open the target conversation on the desktop.");
  }
  await sendRemotePrompt(text, attachments);
}

async function resolvePermission(command: CodeRemoteCommand): Promise<void> {
  const state = requireActiveDesktop(command);
  const actionId = commandString(command, "action_id");
  const runId = commandRunId(command);
  const approved = commandBool(command, "approved");
  const pending = state.snapshot.pending_permission;
  const run = runId ? state.snapshot.runs[runId] : null;
  if (!actionId || !runId || approved === null) {
    throw new Error("Clark Code permission command is incomplete.");
  }
  if (!run || run.status !== "awaiting_input") {
    throw new Error("That run is not waiting for mobile approval.");
  }
  if (pending?.session !== command.desktop_id) {
    throw new Error("That permission request belongs to another desktop conversation.");
  }
  if (!pending || pending.id !== actionId) {
    throw new Error("That permission request is no longer pending.");
  }
  const option = approved
    ? pickAllowOption(pending)
    : pending.options.find((item) => item.kind === "reject_once" || item.kind === "reject_always");
  if (!option) {
    throw new Error("No matching permission option is available.");
  }
  await useSessionStore.getState().resolvePermission(option.id);
}

async function cancelRun(command: CodeRemoteCommand): Promise<void> {
  const state = requireActiveDesktop(command);
  const runId = commandRunId(command);
  const run = runId ? state.snapshot.runs[runId] : null;
  if (!runId) {
    throw new Error("Clark Code cancel command is not bound to a run.");
  }
  if (
    !run ||
    (run.status !== "running" && run.status !== "queued" && run.status !== "awaiting_input")
  ) {
    throw new Error("That run is no longer active.");
  }
  if (!state.bridge || !state.session) {
    throw new Error("Clark Code desktop session is not connected.");
  }
  await state.bridge.cancel(state.session.id, runId);
}

async function runCommand(creds: CloudCreds, hostId: string, command: CodeRemoteCommand): Promise<void> {
  try {
    await ackCodeRemoteCommand(creds, hostId, command.command_id, "accepted", {
      accepted_at: new Date().toISOString(),
      command_type: command.command_type,
    });
    if (command.command_type === "start_session" || command.command_type === "send_message") {
      await submitPrompt(creds, command);
    } else if (command.command_type === "cancel_run") {
      await cancelRun(command);
    } else if (command.command_type === "resolve_permission") {
      await resolvePermission(command);
    } else {
      throw new Error(`Unsupported Clark Code command: ${command.command_type}`);
    }
    const sessionId = useSessionStore.getState().session?.id ?? command.desktop_id ?? null;
    await ackCodeRemoteCommand(creds, hostId, command.command_id, "completed", {
      desktop_id: sessionId,
      submitted_at: new Date().toISOString(),
    });
    void notify("Clark Code", "Mobile command started on this desktop.");
  } catch (error) {
    await ackCodeRemoteCommand(creds, hostId, command.command_id, "failed", {
      error: String(error).slice(0, 800),
      failed_at: new Date().toISOString(),
    }).catch(() => undefined);
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
    let stopped = false;
    let authRefresh: Promise<void> | null = null;
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
            await registerCodeRemoteHost(creds, {
              hostId,
              displayName: `${auth.user.name || "Clark"} desktop`,
              os: navigator.platform || "desktop",
              arch: "",
              appVersion: "desktop",
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
        const response = await pollCodeRemoteCommands(creds, hostId, 20, COMMAND_POLL_WAIT_MS);
        for (const command of response.commands) {
          if (stopped) break;
          await runCommand(creds, hostId, command);
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
    window.addEventListener("online", resumeAfterOutage);
    return () => {
      stopped = true;
      presenceLoop.stop();
      window.clearInterval(timer);
      window.removeEventListener("online", resumeAfterOutage);
    };
  }, [auth, cwd]);

  return null;
}
