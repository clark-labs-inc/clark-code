import type { Session } from "../core-bridge/types";

/** The persisted conversation project wins over the mutable new-session
 * default. Legacy conversations without project metadata use the default. */
export function conversationProjectRoot(
  persistedProject: string | undefined,
  defaultProject: string,
): string {
  return persistedProject?.trim() || defaultProject.trim();
}

/** Resolve the authoritative root for one live conversation. Native providers
 * report the canonical checkout; the host-captured root covers older/provider
 * sessions that do not yet expose an environment. */
export function liveProjectRoot(
  session: Session,
  capturedProject: string | null,
): string | null {
  return session.environment?.checkout_root?.trim() || capturedProject?.trim() || null;
}
