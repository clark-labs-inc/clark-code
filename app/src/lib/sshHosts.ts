// Saved SSH targets for remote projects, persisted per-app. Each is a connection
// preset the user picks when starting a remote coding session; the desktop then
// attaches to one native-owned durable worker on that host.
//
// Nothing secret is stored here — auth is the user's own SSH (keys, agent,
// ~/.ssh/config). `host` is passed verbatim to `ssh`, so a config alias or
// `user@host` both work.

import { accountScopedKey } from "./accountProjectStorage";

export interface SshHost {
  id: string;
  /** Display label (defaults to the host if left blank). */
  label: string;
  /** ssh destination: a ~/.ssh/config alias or `user@host`. */
  host: string;
  /** Absolute project root on the remote. */
  remoteRoot: string;
}

const KEY = "clark-desktop:ssh-hosts";

export function loadSshHosts(scope?: string | null): SshHost[] {
  try {
    const raw = localStorage.getItem(accountScopedKey(KEY, scope));
    const list = raw ? (JSON.parse(raw) as Partial<SshHost>[]) : [];
    if (!Array.isArray(list)) return [];
    return list.flatMap((host) =>
      typeof host.id === "string"
      && typeof host.label === "string"
      && typeof host.host === "string"
      && typeof host.remoteRoot === "string"
        ? [{ id: host.id, label: host.label, host: host.host, remoteRoot: host.remoteRoot }]
        : []
    );
  } catch {
    return [];
  }
}

export function saveSshHosts(list: SshHost[], scope?: string | null): void {
  try {
    localStorage.setItem(accountScopedKey(KEY, scope), JSON.stringify(list));
  } catch {
    /* quota — best effort */
  }
}

export function blankHost(): SshHost {
  return { id: crypto.randomUUID(), label: "", host: "", remoteRoot: "" };
}

/** Return the most recently added host, if this edit introduced one. */
export function newlyAddedSshHostId(current: SshHost[], saved: SshHost[]): string | null {
  const savedIds = new Set(saved.map((host) => host.id));
  return current.findLast((host) => !savedIds.has(host.id))?.id ?? null;
}

/** Everything a host needs to connect is filled in. The binary path is optional
 *  (the server is fetched from the CDN), so only host + folder are required. */
export function hostReady(h: SshHost): boolean {
  return Boolean(h.host.trim() && h.remoteRoot.trim());
}

/** The label to show for a host (falls back to the destination). */
export function hostLabel(h: SshHost): string {
  return h.label.trim() || h.host.trim() || "Untitled host";
}
