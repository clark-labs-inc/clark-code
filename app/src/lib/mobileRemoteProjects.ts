import { useSessionStore } from "../store/sessionStore";
import { liveSessions } from "../store/sessionStore.runtime";
import type { CodeRemoteCommand, CodeRemoteProjectRegistration } from "./mobileRemote";
import { codeKeyAccountBinding } from "./account";
import { loadSshHosts, hostLabel, hostReady } from "./sshHosts";
import { repositoryIdentityForRoot, repositoriesUnderRoot } from "./repositoryKnowledge";
import { remoteFailure } from "./mobileRemoteFailure";

function leaf(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() || path || "Project";
}

function localProjectId(root: string): string {
  return `local:${encodeURIComponent(root)}`;
}

function sshProjectId(hostId: string, root: string): string {
  return `ssh:${encodeURIComponent(hostId)}:${encodeURIComponent(root)}`;
}

export function currentProjects(): CodeRemoteProjectRegistration[] {
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
  const accountScope = codeKeyAccountBinding(useSessionStore.getState().auth);
  for (const host of loadSshHosts(accountScope).filter(hostReady)) {
    projects.push({
      id: sshProjectId(host.id, host.remoteRoot.trim()),
      kind: "ssh",
      display_name: hostLabel(host),
      root: host.remoteRoot.trim(),
      ssh_alias: host.host.trim(),
      trusted: true,
    });
  }
  const roots = new Set([
    ...state.recentProjects,
    ...[...liveSessions.values()].filter((entry) => !entry.remoteHost).map((entry) => entry.projectRoot ?? ""),
  ]);
  for (const root of roots) {
    if (!root.trim() || projects.some((project) => project.kind === "local" && project.root === root)) continue;
    projects.push({
      id: localProjectId(root), kind: "local", display_name: leaf(root), root, trusted: true,
      repository_fingerprint: repositoryIdentityForRoot(root)?.fingerprint ?? null,
    });
  }
  return projects;
}

function projectFor(command: CodeRemoteCommand): CodeRemoteProjectRegistration | null {
  if (!command.project_id) return null;
  return currentProjects().find((project) => project.id === command.project_id) ?? null;
}

export async function applyProject(command: CodeRemoteCommand): Promise<void> {
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
  const host = loadSshHosts(codeKeyAccountBinding(state.auth)).find((item) => sshProjectId(item.id, item.remoteRoot.trim()) === project.id);
  if (!host) {
    remoteFailure(
      "project_unavailable",
      "SSH project is no longer configured on this desktop host.",
    );
  }
  state.setProjectMode("remote");
  state.setSelectedHostId(host.id);
}


/** Start through the same project/session actions used by the desktop UI. */
export async function startMobileRemoteSession(command: CodeRemoteCommand): Promise<void> {
  const target = command.request.session_target as
    | { kind: "new_project" }
    | { kind: "conversation"; conversation_id: string }
    | null | undefined;
  if (target && command.project_id) remoteFailure("invalid_command", "Choose one session destination.");
  if (target?.kind === "new_project") {
    await useSessionStore.getState().startQuickChat();
  } else if (target?.kind === "conversation") {
    if (!target.conversation_id?.trim()) remoteFailure("invalid_command", "Choose a project conversation.");
    await useSessionStore.getState().openConversation(target.conversation_id);
    const state = useSessionStore.getState();
    if (state.session?.id !== target.conversation_id || !state.activeProjectRoot) {
      remoteFailure("project_unavailable", "This project's folder is not available on this computer.");
    }
    if (state.activeRemoteHost) {
      const host = loadSshHosts(codeKeyAccountBinding(state.auth)).find((item) => item.host.trim() === state.activeRemoteHost);
      if (!host) remoteFailure("project_unavailable", "This project's SSH connection is unavailable.");
      await state.startNewProject({ kind: "remote", host: { ...host, remoteRoot: state.activeProjectRoot } });
    } else {
      await state.startNewProject({ kind: "local", path: state.activeProjectRoot, base: "current" });
    }
  } else if (target) {
    remoteFailure("invalid_command", "Unknown session destination.");
  } else {
    await applyProject(command);
    useSessionStore.getState().endSession();
    await useSessionStore.getState().startSession();
  }
  const state = useSessionStore.getState();
  if (!state.session || state.error) remoteFailure("project_unavailable", "The project session could not start.");
}
