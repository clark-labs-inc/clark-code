// Project file list for the `@`-mention picker, fetched from the native bridge
// and cached per folder (the walk is cheap but not free, and the list is stable
// within a session). Returns an empty list when unsupported (browser preview).

import { getBridge } from "../core-bridge/bridge";

const cache = new Map<string, string[]>();
const inflight = new Map<string, Promise<string[]>>();

/** Project-relative file paths under `cwd`, cached. Empty if the bridge can't
 *  list files (e.g. browser preview / no folder). */
export async function projectFiles(
  cwd: string,
  remote?: { id: string } | null,
): Promise<string[]> {
  const root = cwd.trim();
  if (!root) return [];
  const key = remote ? `${remote.id}\0${root}` : root;
  const cached = cache.get(key);
  if (cached) return cached;
  const pending = inflight.get(key);
  if (pending) return pending;

  const load = (async () => {
    try {
      const bridge = await getBridge();
      const files = (await bridge.listFiles?.(root, remote)) ?? [];
      cache.set(key, files);
      return files;
    } catch {
      return [];
    } finally {
      inflight.delete(key);
    }
  })();
  inflight.set(key, load);
  return load;
}
