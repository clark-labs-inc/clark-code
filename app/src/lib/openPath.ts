// Open a project file in the OS default app (typically the user's editor) or
// reveal it in the file manager, via the native bridge. No-op in the browser.

import { getBridge } from "../core-bridge/bridge";

function joinPath(cwd: string, rel: string): string {
  if (rel.startsWith("/")) return rel; // already absolute
  return `${cwd.replace(/[/\\]+$/, "")}/${rel.replace(/^[/\\]+/, "")}`;
}

/** Open (or reveal) a project-relative path rooted at `cwd`. */
export async function openProjectPath(cwd: string, rel: string, reveal = false): Promise<void> {
  const path = joinPath(cwd, rel);
  try {
    const bridge = await getBridge();
    await bridge.openPath?.(path, reveal);
  } catch {
    /* not supported (browser) or failed — non-fatal */
  }
}
