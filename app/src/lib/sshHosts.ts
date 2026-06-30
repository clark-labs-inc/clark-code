// Saved SSH targets for remote projects, persisted per-app. Each is a connection
// preset the user picks when starting a remote coding session; the desktop then
// brings up a clark-exec-server on that host and tunnels to it (see ssh.ts).
//
// Nothing secret is stored here — auth is the user's own SSH (keys, agent,
// ~/.ssh/config). `host` is passed verbatim to `ssh`, so a config alias or
// `user@host` both work.

export interface SshHost {
  id: string;
  /** Display label (defaults to the host if left blank). */
  label: string;
  /** ssh destination: a ~/.ssh/config alias or `user@host`. */
  host: string;
  /** Absolute project root on the remote. */
  remoteRoot: string;
  /**
   * Local path to a `clark-exec-server` built for the remote's architecture.
   * Dev-only: the desktop uploads it on connect. Goes away once the binary is
   * fetched from the downloads CDN (server-side feature, later phase).
   */
  binaryPath: string;
}

const KEY = "clark-desktop:ssh-hosts";

export function loadSshHosts(): SshHost[] {
  try {
    const raw = localStorage.getItem(KEY);
    const list = raw ? (JSON.parse(raw) as SshHost[]) : [];
    return Array.isArray(list) ? list : [];
  } catch {
    return [];
  }
}

export function saveSshHosts(list: SshHost[]): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(list));
  } catch {
    /* quota — best effort */
  }
}

export function blankHost(): SshHost {
  return { id: crypto.randomUUID(), label: "", host: "", remoteRoot: "", binaryPath: "" };
}

/** Everything a host needs to connect is filled in. */
export function hostReady(h: SshHost): boolean {
  return Boolean(h.host.trim() && h.remoteRoot.trim() && h.binaryPath.trim());
}

/** The label to show for a host (falls back to the destination). */
export function hostLabel(h: SshHost): string {
  return h.label.trim() || h.host.trim() || "Untitled host";
}
