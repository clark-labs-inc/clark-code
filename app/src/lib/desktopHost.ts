const HOST_ID_KEY = "clark-desktop:code-remote-host-id";
const DESKTOP_INSTANCE_ID = (() => {
  try {
    return crypto.randomUUID();
  } catch {
    return `desktop-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  }
})();

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

/** Ephemeral process identity used to fence remote-command execution leases. */
export function desktopInstanceId(): string {
  return DESKTOP_INSTANCE_ID;
}
