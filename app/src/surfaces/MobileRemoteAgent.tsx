import { useEffect, useRef } from "react";
import { useSessionStore } from "../store/sessionStore";
import { cloudCreds, type CloudCreds } from "../lib/cloudHistory";
import {
  ackCodeRemoteCommand,
  pollCodeRemoteCommands,
  registerCodeRemoteHost,
  type CodeRemoteCommand,
  type CodeRemoteProjectRegistration,
} from "../lib/mobileRemote";
import { loadSshHosts, hostLabel, hostReady } from "../lib/sshHosts";
import { pickAllowOption } from "../lib/permissions";
import { notify } from "../lib/notify";
import { isAuthExpiredError, refreshAuthSession } from "../lib/auth";

const HOST_ID_KEY = "clark-desktop:code-remote-host-id";
const LOOP_INTERVAL_MS = 500;
const HOST_HEARTBEAT_INTERVAL_MS = 30_000;
const COMMAND_POLL_WAIT_MS = 25_000;

function getHostId(): string {
  try {
    const existing = localStorage.getItem(HOST_ID_KEY);
    if (existing) return existing;
    const next = crypto.randomUUID();
    localStorage.setItem(HOST_ID_KEY, next);
    return next;
  } catch {
    return "desktop";
  }
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
    projects.push({
      id: localProjectId(cwd),
      kind: "local",
      display_name: leaf(cwd),
      root: cwd,
      trusted: true,
    });
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

async function submitPrompt(command: CodeRemoteCommand): Promise<void> {
  const text = commandText(command);
  if (!text) {
    throw new Error("Clark Code command has no prompt text.");
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
    // openConversation degrades to a read-only peek while another
    // conversation is mid-run, so the session never bound to the target.
    // (Follow-ups to the RUNNING conversation itself don't hit this — they
    // bind fine and send() queues them until the run settles.)
    throw new Error(
      "The desktop is busy with a different conversation. Wait for it to finish, then try again.",
    );
  }
  await useSessionStore.getState().send(text);
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
      await submitPrompt(command);
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
  const busyRef = useRef(false);
  const lastHeartbeatRef = useRef(0);

  useEffect(() => {
    if (!auth) return;
    const hostId = getHostId();
    let stopped = false;
    lastHeartbeatRef.current = 0;

    const tick = async () => {
      if (stopped || busyRef.current) return;
      const creds = cloudCreds(useSessionStore.getState().auth);
      if (!creds) return;
      busyRef.current = true;
      try {
        const now = Date.now();
        if (lastHeartbeatRef.current === 0 || now - lastHeartbeatRef.current >= HOST_HEARTBEAT_INTERVAL_MS) {
          await registerCodeRemoteHost(creds, {
            hostId,
            displayName: `${auth.user.name || "Clark"} desktop`,
            os: navigator.platform || "desktop",
            arch: "",
            appVersion: "desktop",
            projects: currentProjects(),
          });
          lastHeartbeatRef.current = Date.now();
        }
        const response = await pollCodeRemoteCommands(creds, hostId, 20, COMMAND_POLL_WAIT_MS);
        for (const command of response.commands) {
          if (stopped) break;
          await runCommand(creds, hostId, command);
        }
      } catch (error) {
        if (isAuthExpiredError(error)) {
          const currentAuth = useSessionStore.getState().auth;
          const refreshed = currentAuth ? await refreshAuthSession(currentAuth) : null;
          if (refreshed) {
            useSessionStore.setState({ auth: refreshed });
          } else {
            useSessionStore.getState().signOutAuth();
            void notify("Clark sign-in expired", "Sign in again to keep Clark Code remote control online.");
          }
        }
        /* Remote control is a background affordance; normal desktop use continues. */
      } finally {
        busyRef.current = false;
      }
    };

    void tick();
    const timer = window.setInterval(() => void tick(), LOOP_INTERVAL_MS);
    return () => {
      stopped = true;
      window.clearInterval(timer);
    };
  }, [auth, cwd]);

  return null;
}
