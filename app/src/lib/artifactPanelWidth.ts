export const ARTIFACT_PANEL_WIDTH_KEY = "clark.artifact-panel-width";
export const DEFAULT_ARTIFACT_PANEL_WIDTH = 640;
export const MIN_ARTIFACT_PANEL_WIDTH = 420;
export const MIN_CONVERSATION_PANEL_WIDTH = 320;
const MAX_STORED_ARTIFACT_PANEL_WIDTH = 1600;

interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

function defaultStorage(): StorageLike | undefined {
  return typeof localStorage === "undefined" ? undefined : localStorage;
}

export function constrainArtifactPanelWidth(width: number, containerWidth: number): number {
  const requested = Number.isFinite(width) ? width : DEFAULT_ARTIFACT_PANEL_WIDTH;
  const available = Number.isFinite(containerWidth) ? Math.max(0, containerWidth) : 0;
  const maximum = Math.max(0, available - MIN_CONVERSATION_PANEL_WIDTH);
  const minimum = Math.min(MIN_ARTIFACT_PANEL_WIDTH, maximum);
  return Math.round(Math.min(Math.max(requested, minimum), maximum));
}

export function loadArtifactPanelWidth(storage: StorageLike | undefined = defaultStorage()): number {
  if (!storage) return DEFAULT_ARTIFACT_PANEL_WIDTH;
  try {
    const value = Number(storage.getItem(ARTIFACT_PANEL_WIDTH_KEY));
    if (!Number.isFinite(value) || value <= 0) return DEFAULT_ARTIFACT_PANEL_WIDTH;
    return Math.round(Math.min(Math.max(value, MIN_ARTIFACT_PANEL_WIDTH), MAX_STORED_ARTIFACT_PANEL_WIDTH));
  } catch {
    return DEFAULT_ARTIFACT_PANEL_WIDTH;
  }
}

export function saveArtifactPanelWidth(
  width: number,
  storage: StorageLike | undefined = defaultStorage(),
): void {
  if (!storage || !Number.isFinite(width)) return;
  try {
    storage.setItem(ARTIFACT_PANEL_WIDTH_KEY, String(Math.round(width)));
  } catch {
    // A locked-down WebView may deny localStorage. The live resize still works.
  }
}
