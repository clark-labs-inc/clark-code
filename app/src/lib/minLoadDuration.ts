// Keep a loading state "true" for at least `ms` so a fast operation (a local
// disk read) still paints its spinner. Without this, `set(loading=true)` and
// `set(loading=false)` land in the same React render batch and the spinner is
// never painted — the icon looks frozen instead of spinning. Slower work
// (downloads, agent runs, SSH) keeps loading true long enough naturally.
//
// await minLoadDuration(work, 250) guarantees the promise settles no faster
// than `ms` after the call (happy or error path), but never slower than `work`
// itself.

const MIN_SPIN_MS = 250;

/** Wait for `work` to settle AND for `ms` to elapse, then return `work`'s
 *  value (re-throwing if it rejected). Use for loading-flag-gated refreshes so
 *  the spinner paints at least one full spin even when the read is instant. */
export async function minLoadDuration<T>(work: Promise<T>, ms = MIN_SPIN_MS): Promise<T> {
  const timer = new Promise<void>((resolve) => setTimeout(resolve, ms));
  // allSettled: a fast rejection still waits for the floor so the loading flag
  // is held for the same duration in both paths.
  await Promise.allSettled([work, timer]);
  return work; // async functions flatten the returned promise: surfaces work's rejection
}
