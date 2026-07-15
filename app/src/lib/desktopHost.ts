const HOST_ID_KEY = "clark-desktop:code-remote-host-id";

/** Stable installation-local identifier used for host leases and contribution sources. */
export function desktopHostId(): string {
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
