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

/** Byte progress while a staged update downloads. `total` is null until the
 *  server reports a content length. */
export interface DownloadProgress {
  downloaded: number;
  total: number | null;
}

/** Check for an update and, if one exists, download + verify + stage it,
 *  reporting byte progress via `onProgress`. Returns the staged version (ready
 *  to apply on relaunch), or null. Never throws. */
export async function checkAndStageUpdate(
  onProgress?: (p: DownloadProgress) => void,
): Promise<StagedUpdate | null> {
  if (!inTauri()) return null;
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    if (!update) return null;
    // Downloads, verifies the Ed25519 signature, and stages the new bundle.
    // Throws if the signature doesn't match the embedded pubkey.
    let total: number | null = null;
    let downloaded = 0;
    await update.downloadAndInstall((e) => {
      if (e.event === "Started") {
        total = e.data.contentLength ?? null;
        downloaded = 0;
      } else if (e.event === "Progress") {
        downloaded += e.data.chunkLength;
      } else if (e.event === "Finished") {
        downloaded = total ?? downloaded;
      }
      onProgress?.({ downloaded, total });
    });
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

// The running version is recorded on every launch; when it differs from the last
// recorded one, the app came back on a freshly-applied update — surface a
// one-time confirmation so the restart doesn't feel like a black box.
const LAST_VERSION_KEY = "clark.lastAppVersion";

/** On the first launch after an update, return the new version (once) and record
 *  it; otherwise null. Never fires on a fresh install (no prior version). */
export async function consumeJustUpdated(): Promise<string | null> {
  if (!inTauri()) return null;
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    const current = await getVersion();
    let prev: string | null = null;
    try {
      prev = localStorage.getItem(LAST_VERSION_KEY);
      localStorage.setItem(LAST_VERSION_KEY, current);
    } catch {
      /* localStorage unavailable — skip the confirmation */
    }
    return prev && prev !== current ? current : null;
  } catch {
    return null;
  }
}
