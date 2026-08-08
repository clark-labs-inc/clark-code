import {
  fetchSnapshot,
  liveSessions,
  mergedOf,
} from "../store/sessionStore.runtime";
import { useSessionStore } from "../store/sessionStore";
import { authAccountMatches } from "./account";

const OPEN_WAIT_INTERVAL_MS = 1_000;
const OPEN_WAIT_TIMEOUT_MS = 30_000;

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => {
    globalThis.setTimeout(resolve, ms);
  });
}

/**
 * Make a phone-targeted conversation live before executing an action against
 * its run state. A desktop restart intentionally begins with an empty live
 * session pool; opening the conversation is also the boundary that replays the
 * native trajectory journal and publishes interrupted runs back to mobile.
 */
export async function ensureMobileRemoteLiveTarget(desktopId: string) {
  let entry = liveSessions.get(desktopId);
  if (entry) return entry;

  const deadline = Date.now() + OPEN_WAIT_TIMEOUT_MS;
  const openingSameTarget = useSessionStore.getState().opening?.id === desktopId;
  if (!openingSameTarget) {
    await Promise.race([
      useSessionStore.getState().openConversation(desktopId),
      wait(OPEN_WAIT_TIMEOUT_MS),
    ]);
  }

  while (Date.now() < deadline) {
    entry = liveSessions.get(desktopId);
    if (entry) return entry;
    if (useSessionStore.getState().opening?.id !== desktopId) break;
    await wait(Math.min(OPEN_WAIT_INTERVAL_MS, deadline - Date.now()));
  }
  return null;
}

/**
 * Read the recovered authoritative snapshot before paying the cost of opening
 * a provider session. Most phone Stop/Steer actions arriving after a desktop
 * restart are already stale; recovery can settle them without reconnecting a
 * large transcript or moving the laptop UI.
 */
export async function inspectMobileRemoteTarget(desktopId: string) {
  const entry = liveSessions.get(desktopId);
  if (entry) return { entry, snapshot: mergedOf(entry) };
  const requestAuth = useSessionStore.getState().auth;
  const snapshot = await fetchSnapshot(
    desktopId,
    requestAuth,
    () => authAccountMatches(requestAuth, useSessionStore.getState().auth),
  );
  return { entry: null, snapshot };
}
