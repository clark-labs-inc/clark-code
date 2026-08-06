import {
  getBridge,
  type ProjectContext,
  type RemoteWorkerTarget,
} from "../core-bridge/bridge";

/** Fetch the current checkout identity. A missing bridge method, non-Git
 * folder, or failed best-effort probe all resolve to null so the composer never
 * blocks on status chrome. */
export async function loadProjectContext(
  cwd: string,
  remote?: RemoteWorkerTarget | null,
): Promise<ProjectContext | null> {
  const root = cwd.trim();
  if (!root) return null;

  try {
    const bridge = await getBridge();
    return (await bridge.projectContext?.(root, remote)) ?? null;
  } catch {
    return null;
  }
}
