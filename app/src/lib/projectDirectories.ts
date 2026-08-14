import { getBridge, type ProjectDirectory } from "../core-bridge/bridge";

const cache = new Map<string, ProjectDirectory[]>();

/** Immediate sibling folders for repository autocomplete. Unsupported and
 * inaccessible parents simply contribute no suggestions. */
export async function siblingProjectDirectories(
  cwd: string,
  remote?: { id: string } | null,
): Promise<ProjectDirectory[]> {
  const root = cwd.trim();
  if (!root) return [];
  const key = remote ? `${remote.id}\0${root}` : root;
  const cached = cache.get(key);
  if (cached) return cached;
  try {
    const bridge = await getBridge();
    const directories = (await bridge.listSiblingDirectories?.(root, remote)) ?? [];
    cache.set(key, directories);
    return directories;
  } catch {
    return [];
  }
}
