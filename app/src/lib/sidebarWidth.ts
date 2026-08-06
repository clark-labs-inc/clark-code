// Persisted width for the resizable left conversation sidebar.
//
// The sidebar starts at 17rem like the old fixed layout, can be dragged
// narrower (rows truncate) or wider (up to a stored cap), and always leaves
// enough room for the conversation pane by clamping against the current window
// width. Mirrors the artifact-panel width helpers in artifactPanelWidth.ts.

export const SIDEBAR_WIDTH_KEY = "clark.sidebar-width";
export const DEFAULT_SIDEBAR_WIDTH = 272; // 17rem — the historical fixed width
export const MIN_SIDEBAR_WIDTH = 200;
/** Reserved for the conversation pane when the sidebar is constrained by the window. */
export const MIN_CONVERSATION_PANEL_WIDTH = 360;
const MAX_STORED_SIDEBAR_WIDTH = 640;

interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

function defaultStorage(): StorageLike | undefined {
  return typeof localStorage === "undefined" ? undefined : localStorage;
}

export function constrainSidebarWidth(width: number, windowWidth?: number): number {
  const requested = Number.isFinite(width) ? width : DEFAULT_SIDEBAR_WIDTH;
  const available = Number.isFinite(windowWidth) ? Math.max(0, windowWidth ?? 0) : 0;
  const maximum =
    available > 0
      ? Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_STORED_SIDEBAR_WIDTH, available - MIN_CONVERSATION_PANEL_WIDTH))
      : MAX_STORED_SIDEBAR_WIDTH;
  const minimum = Math.min(MIN_SIDEBAR_WIDTH, maximum);
  return Math.round(Math.min(Math.max(requested, minimum), maximum));
}

export function loadSidebarWidth(storage: StorageLike | undefined = defaultStorage()): number {
  if (!storage) return DEFAULT_SIDEBAR_WIDTH;
  try {
    const value = Number(storage.getItem(SIDEBAR_WIDTH_KEY));
    if (!Number.isFinite(value) || value <= 0) return DEFAULT_SIDEBAR_WIDTH;
    return Math.round(Math.min(Math.max(value, MIN_SIDEBAR_WIDTH), MAX_STORED_SIDEBAR_WIDTH));
  } catch {
    return DEFAULT_SIDEBAR_WIDTH;
  }
}

export function saveSidebarWidth(
  width: number,
  storage: StorageLike | undefined = defaultStorage(),
): void {
  if (!storage || !Number.isFinite(width)) return;
  try {
    storage.setItem(SIDEBAR_WIDTH_KEY, String(Math.round(width)));
  } catch {
    // A locked-down WebView may deny localStorage. The live resize still works.
  }
}
