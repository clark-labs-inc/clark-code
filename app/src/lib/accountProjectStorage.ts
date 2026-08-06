const PROJECT_CONTEXT_KEY_PREFIX = "clark-desktop:project-context:";
const RECENTS_KEY = "clark-desktop:recent-projects";
const ANONYMOUS_SCOPE = "anonymous";
const MAX_RECENTS = 8;

export function normalizedAccountScope(scope: string | null | undefined): string | null {
  if (scope == null) return null;
  const normalized = scope.trim().toLowerCase();
  return normalized ? normalized : null;
}

/**
 * Return a storage key isolated for an authenticated account or the signed-out
 * anonymous state. An unscoped storage namespace does not exist.
 */
export function accountScopedKey(
  base: string,
  scope: string | null | undefined,
): string {
  const normalized = normalizedAccountScope(scope);
  return `${base}:${encodeURIComponent(normalized ?? ANONYMOUS_SCOPE)}`;
}

function projectContextKey(scope: string): string {
  return `${PROJECT_CONTEXT_KEY_PREFIX}${encodeURIComponent(scope)}`;
}

export function loadProjectCwd(
  scope: string | null | undefined,
): string {
  if (!scope) return "";
  try {
    const raw = localStorage.getItem(projectContextKey(scope));
    if (raw) {
      const parsed = JSON.parse(raw) as { cwd?: unknown };
      if (typeof parsed.cwd === "string") return parsed.cwd;
    }
  } catch {
    /* localStorage can be unavailable or contain a malformed old value. */
  }
  return "";
}

export function saveProjectCwd(scope: string | null | undefined, cwd: string): void {
  const normalized = normalizedAccountScope(scope);
  if (!normalized) return;
  try {
    localStorage.setItem(projectContextKey(normalized), JSON.stringify({ cwd }));
  } catch {
    /* Non-fatal. The in-memory project remains usable for this session. */
  }
}

function recentProjectsKey(scope: string | null | undefined): string {
  const normalized = normalizedAccountScope(scope);
  return `${RECENTS_KEY}:${encodeURIComponent(normalized ?? ANONYMOUS_SCOPE)}`;
}

/** Most-recently-used project folders, newest first. */
export function loadRecentProjects(scope?: string | null): string[] {
  try {
    const raw = localStorage.getItem(recentProjectsKey(scope));
    if (!raw) return [];
    const list = JSON.parse(raw) as unknown;
    return Array.isArray(list) ? list.filter((p): p is string => typeof p === "string") : [];
  } catch {
    return [];
  }
}

/** Push `path` to the front of the recents list (de-duped), and persist. */
export function addRecentProject(path: string, scope?: string | null): string[] {
  const clean = path.trim();
  if (!clean) return loadRecentProjects(scope);
  const next = [clean, ...loadRecentProjects(scope).filter((p) => p !== clean)].slice(0, MAX_RECENTS);
  try {
    localStorage.setItem(recentProjectsKey(scope), JSON.stringify(next));
  } catch {
    // Non-fatal.
  }
  return next;
}

/** Forget one folder from the project list without touching its files or chats. */
export function removeRecentProject(path: string, scope?: string | null): string[] {
  const next = loadRecentProjects(scope).filter((candidate) => candidate !== path.trim());
  try {
    localStorage.setItem(recentProjectsKey(scope), JSON.stringify(next));
  } catch {
    // Non-fatal.
  }
  return next;
}
