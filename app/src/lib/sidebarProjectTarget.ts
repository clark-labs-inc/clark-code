import type { ProjectGroup } from "./projectSidebar";
import type { SshHost } from "./sshHosts";

/** A project row names an exact folder, even when its saved host default changed. */
export function sidebarProjectHost(group: ProjectGroup, hosts: SshHost[]): SshHost | null {
  const candidates = hosts.filter((host) => host.host.trim() === group.remoteHost?.trim());
  const host = candidates.find((candidate) => candidate.remoteRoot.trim() === group.remoteRoot?.trim()) ?? candidates[0];
  if (!host) return null;
  return { ...host, remoteRoot: group.remoteRoot?.trim() || host.remoteRoot };
}
