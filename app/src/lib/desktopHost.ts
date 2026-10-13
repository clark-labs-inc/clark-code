import { invoke } from "@tauri-apps/api/core";

let identity: Promise<{ id: string; name: string }> | undefined;
export function desktopIdentity(): Promise<{ id: string; name: string }> {
  return identity ??= invoke<{ id: string; name: string }>("desktop_identity").catch((error) => {
    identity = undefined;
    throw error;
  });
}

const DESKTOP_INSTANCE_ID = (() => {
  try {
    return crypto.randomUUID();
  } catch {
    return `desktop-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  }
})();

/** Ephemeral process identity used to fence remote-command execution leases. */
export function desktopInstanceId(): string {
  return DESKTOP_INSTANCE_ID;
}
