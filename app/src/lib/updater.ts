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
import type { Update } from "@tauri-apps/plugin-updater";

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function updatesEnabled(): Promise<boolean> {
  if (!inTauri()) return false;
  try {
    return await invoke<boolean>("app_updates_enabled");
  } catch {
    return false;
  }
}

export interface StagedUpdate {
  version: string;
  notes?: string;
}

export type UpdateCheckResult =
  | { status: "ready"; update: StagedUpdate }
  | { status: "up-to-date" }
  | { status: "busy" }
  | { status: "unavailable" }
  | { status: "error"; message: string };

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

// latest.json is the sole mutable object in the update channel. Ask every check
// to revalidate it so a long-lived CDN/browser cache cannot pin an older release
// after the user has already chosen "Restart to update".
function revalidateOptions() {
  // The Tauri JS plugin normalizes headers by mutating the supplied options
  // object, so return a fresh object for every request.
  return {
    headers: {
      "Cache-Control": "no-cache, no-store, max-age=0",
      Pragma: "no-cache",
    },
    timeout: 30_000,
  };
}

// If releases land while a large updater payload is downloading, follow the
// channel forward a bounded number of times. A final equal-version check is
// required before install; continuously moving channels fail closed and let the
// user retry instead of knowingly installing a superseded build.
const MAX_SUPERSEDED_DOWNLOADS = 3;

function metadata(update: Update): StagedUpdate {
  return { version: update.version, notes: update.body || undefined };
}

function numericReleaseParts(version: string): number[] | null {
  const normalized = version.trim().replace(/^v/, "");
  if (!/^\d+(?:\.\d+)*$/.test(normalized)) return null;
  return normalized.split(".").map(Number);
}

/** Compare the stable numeric versions used by Clark's production channel. */
function compareReleaseVersions(left: string, right: string): number | null {
  const a = numericReleaseParts(left);
  const b = numericReleaseParts(right);
  if (!a || !b) return null;
  for (let i = 0; i < Math.max(a.length, b.length); i += 1) {
    const delta = (a[i] ?? 0) - (b[i] ?? 0);
    if (delta !== 0) return Math.sign(delta);
  }
  return 0;
}

async function closeQuietly(update: Update | null): Promise<void> {
  if (update) await update.close().catch(() => {});
}

async function downloadUpdate(
  update: Update,
  onProgress?: (p: DownloadProgress) => void,
): Promise<void> {
  let total: number | null = null;
  let downloaded = 0;
  await update.download((e) => {
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
}

/** Check for an update and, if one exists, download + verify + stage it,
 *  reporting byte progress via `onProgress`. The result keeps "up to date"
 *  distinct from transport/signature failures so manual checks never report a
 *  false success. Never throws. */
export async function checkAndStageUpdate(
  onProgress?: (p: DownloadProgress) => void,
): Promise<UpdateCheckResult> {
  if (!(await updatesEnabled())) return { status: "unavailable" };
  let candidate: Update | null = null;
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    candidate = await check(revalidateOptions());
    if (!candidate) return { status: "up-to-date" };

    if (stagedUpdate) {
      const comparison = compareReleaseVersions(candidate.version, stagedUpdate.version);
      if (comparison === null) {
        throw new Error(
          `Cannot safely compare update versions ${candidate.version} and ${stagedUpdate.version}.`,
        );
      }
      if (comparison <= 0) {
        await closeQuietly(candidate);
        candidate = null;
        return { status: "ready", update: metadata(stagedUpdate) };
      }
    }

    // Download + verify only. `install()` is deferred because it forcibly exits
    // on Windows and must therefore sit behind the active-run drain.
    await downloadUpdate(candidate, onProgress);
    const previous = stagedUpdate;
    stagedUpdate = candidate;
    candidate = null;
    await closeQuietly(previous);
    return { status: "ready", update: metadata(stagedUpdate) };
  } catch (error) {
    await closeQuietly(candidate);
    return {
      status: "error",
      message: error instanceof Error ? error.message : String(error),
    };
  }
}

/** Revalidate the mutable latest pointer and replace a superseded staged
 *  download. Call only after the native update drain has reached zero; this
 *  function may download a newer payload and intentionally keeps the latch
 *  held until the caller installs or cancels. */
export async function refreshStagedUpdate(
  onProgress?: (p: DownloadProgress) => void,
): Promise<UpdateCheckResult> {
  if (!inTauri()) return { status: "unavailable" };
  if (!stagedUpdate) {
    return {
      status: "error",
      message: "The downloaded update is no longer available; check again.",
    };
  }

  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    for (let superseded = 0; superseded <= MAX_SUPERSEDED_DOWNLOADS; superseded += 1) {
      let candidate: Update | null = null;
      try {
        candidate = await check(revalidateOptions());
        if (!candidate) {
          throw new Error("The latest update manifest no longer offers the staged version.");
        }

        const comparison = compareReleaseVersions(candidate.version, stagedUpdate.version);
        if (comparison === null) {
          throw new Error(
            `Cannot safely compare update versions ${candidate.version} and ${stagedUpdate.version}.`,
          );
        }
        if (comparison === 0) {
          await closeQuietly(candidate);
          return { status: "ready", update: metadata(stagedUpdate) };
        }
        if (comparison < 0) {
          await closeQuietly(candidate);
          if (superseded === MAX_SUPERSEDED_DOWNLOADS) {
            throw new Error(
              `The update channel returned older version ${candidate.version} after ${stagedUpdate.version}; retry after the CDN refreshes.`,
            );
          }
          continue;
        }
        if (superseded === MAX_SUPERSEDED_DOWNLOADS) {
          await closeQuietly(candidate);
          throw new Error("The update channel changed repeatedly; retry to install the latest release.");
        }

        await downloadUpdate(candidate, onProgress);
        const previous = stagedUpdate;
        stagedUpdate = candidate;
        candidate = null;
        await closeQuietly(previous);
        // Check once more. Installation is allowed only when the mutable pointer
        // returns the exact version whose signed bytes are currently staged.
      } catch (error) {
        await closeQuietly(candidate);
        throw error;
      }
    }
    throw new Error("The update channel did not stabilize.");
  } catch (error) {
    return {
      status: "error",
      message: error instanceof Error ? error.message : String(error),
    };
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
