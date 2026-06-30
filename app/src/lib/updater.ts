// Secure in-app auto-update (desktop only).
//
// Best-practice flow: on launch (and periodically) check a signed manifest on
// the downloads CDN; if a newer version exists, download + verify + stage it
// silently in the background, then surface a non-blocking "Restart to update"
// affordance. The native binary can't be hot-swapped, so applying the update is
// just a relaunch — which the user does on their own schedule (and which happens
// automatically on the next cold launch if they don't). All signature
// verification is done natively by tauri-plugin-updater against the embedded
// Ed25519 public key; an unsigned or tampered payload is refused.

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export interface StagedUpdate {
  version: string;
  notes?: string;
}

/** Check for an update and, if one exists, download + verify + stage it. Returns
 *  the staged version (ready to apply on relaunch), or null. Never throws. */
export async function checkAndStageUpdate(): Promise<StagedUpdate | null> {
  if (!inTauri()) return null;
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    if (!update) return null;
    // Downloads, verifies the Ed25519 signature, and stages the new bundle.
    // Throws if the signature doesn't match the embedded pubkey.
    await update.downloadAndInstall();
    return { version: update.version, notes: update.body || undefined };
  } catch {
    // Offline, no manifest yet, or verification failed — stay on the current
    // version silently; we'll retry on the next check.
    return null;
  }
}

/** Relaunch into the staged update. No-op outside the desktop app. */
export async function relaunchApp(): Promise<void> {
  if (!inTauri()) return;
  try {
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
  } catch {
    /* ignore */
  }
}
