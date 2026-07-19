// Secure in-app auto-update (desktop only).
//
// Best-practice flow: on launch (and periodically) check a signed manifest on
// the downloads CDN; if a newer version exists, download + verify + stage it
// silently in the background, then surface a non-blocking "Restart to update"
// affordance. Download and install are deliberately separate: installing a
// Tauri update exits the app immediately on Windows, so even the install step
// must wait for active coding work to drain. All signature verification is done
// natively by tauri-plugin-updater against the embedded Ed25519 public key; an
// unsigned or tampered payload is refused.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Update } from "@tauri-apps/plugin-updater";

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

// The updater plugin keeps downloaded bytes in a native resource owned by this
// Update object. It intentionally lives outside reactive state; the UI only
// needs serializable metadata while this handle waits for the drain gate.
let stagedUpdate: Update | null = null;

/** Check for an update and, if one exists, download + verify + stage it,
 *  reporting byte progress via `onProgress`. Returns the staged version (ready
 *  to apply on relaunch), or null. Never throws. */
export async function checkAndStageUpdate(
  onProgress?: (p: DownloadProgress) => void,
): Promise<StagedUpdate | null> {
  if (!inTauri()) return null;
  let candidate: Update | null = null;
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    candidate = await check();
    if (!candidate) return null;
    // Download + verify only. `install()` is deferred because it forcibly exits
    // on Windows and must therefore sit behind the active-run drain.
    let total: number | null = null;
    let downloaded = 0;
    await candidate.download((e) => {
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
    stagedUpdate = candidate;
    return { version: candidate.version, notes: candidate.body || undefined };
  } catch {
    if (candidate) void candidate.close().catch(() => {});
    // Offline, no manifest yet, or verification failed — stay on the current
    // version silently; we'll retry on the next check.
    return null;
  }
}

/** Install the already-downloaded update. On Windows this call exits the app;
 *  callers must engage and verify the native drain gate first. */
export async function installStagedUpdate(): Promise<void> {
  if (!inTauri()) return;
  const update = stagedUpdate;
  if (!update) throw new Error("The downloaded update is no longer available; check again.");
  await update.install();
  stagedUpdate = null;
}

/** Engage the native no-new-runs latch and return the exact in-flight count. */
export async function beginUpdateDrain(): Promise<number> {
  if (!inTauri()) return 0;
  return invoke<number>("update_begin_drain");
}

/** Release the native latch after an install/relaunch failure. */
export async function cancelUpdateDrain(): Promise<void> {
  if (!inTauri()) return;
  await invoke("update_cancel_drain");
}

/** Route the native app-menu action through the shared frontend coordinator. */
export async function onUpdateMenuRequested(handler: () => void): Promise<() => void> {
  if (!inTauri()) return () => {};
  return listen("update-menu-requested", handler);
}

/** Relaunch into the staged update. No-op outside the desktop app. */
export async function relaunchApp(): Promise<void> {
  if (!inTauri()) return;
  const { relaunch } = await import("@tauri-apps/plugin-process");
  await relaunch();
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
