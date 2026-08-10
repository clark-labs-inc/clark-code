import type { ConversationMeta } from "./history";
import { projectName } from "./localAgent";

const STORAGE_KEY = "agent-desktop:project-sidebar";
const ANONYMOUS_SCOPE = "anonymous";

function scopedStorageKey(scope: string | null | undefined): string | null {
  if (scope === undefined) return STORAGE_KEY;
  const normalized = scope?.trim().toLowerCase() ?? "";
  return `${STORAGE_KEY}:${encodeURIComponent(normalized || ANONYMOUS_SCOPE)}`;
}

export type ProjectGroupKind = "remote" | "local" | "none";

function quickChatWorkspaceId(path: string): string | null {
  const normalized = path.replaceAll("\\", "/").replace(/\/+$/, "");
  const match = normalized.match(/\/\.agent\/workspace\/([0-9a-f-]{36})$/i);
  return match?.[1] ?? null;
}

/** Quick chats use the agent's per-conversation workspace as their checkout. The
 * suffix is stable across machines even though the home-directory prefix is
 * not, so a cloud-restored chat can recreate its workspace locally. */
export function isQuickChatProject(path: string | undefined, id: string): boolean {
  return path ? quickChatWorkspaceId(path) === id : false;
}

export interface ProjectGroup {
  key: string;
  label: string;
  title: string;
  kind: ProjectGroupKind;
  /** The local folder behind the group (local projects only). */
  path?: string;
  /** Source repository name for a the agent-managed isolated checkout. */
  repositoryLabel?: string;
  /** SSH destination behind a remote group, independent of its display alias. */
  remoteHost?: string;
  /** Remote root associated with the group's first conversation. */
  remoteRoot?: string;
  convos: ConversationMeta[];
  latest: number;
}

export interface ProjectSidebarPreferences {
  pinned: string[];
  aliases: Record<string, string>;
}

const MANAGED_WORKTREE_PATH = /^(.*)\.(?:agent|clark)-worktrees\/([^/]+)$/;

function managedCheckoutParts(path: string): RegExpMatchArray | null {
  return path.replaceAll("\\", "/").replace(/\/+$/, "").match(MANAGED_WORKTREE_PATH);
}

export function remoteProjectKey(host: string, root: string | undefined): string {
  const normalizedHost = host.trim();
  const normalizedRoot = root?.replaceAll("\\", "/").replace(/\/+$/, "") ?? "";
  return `r:${encodeURIComponent(normalizedHost)}:${encodeURIComponent(normalizedRoot)}`;
}

/** Compactly identify a managed isolated checkout without hiding the source
 * repository. Managed worktrees live beside the repository at
 * `<repo>.agent-worktrees/<session>`; showing both names prevents two chats
 * from looking like unrelated projects in the sidebar and recent-work card. */
export function projectDisplayName(path: string): string {
  if (quickChatWorkspaceId(path)) return "Quick Chat";
  const managed = managedCheckoutParts(path);
  if (!managed) return projectName(path);
  return `${projectName(managed[2])} · ${projectName(managed[1])}`;
}

export function projectDisplayTitle(path: string): string {
  if (quickChatWorkspaceId(path)) return "Temporary the agent workspace";
  const managed = managedCheckoutParts(path);
  if (!managed) return path;
  return `${path}\nIsolated checkout of ${projectName(managed[1])}`;
}

/** Return the source repository for a managed checkout. Regular project rows
 * already use the repository name as their primary label, so they need no
 * secondary cue. */
export function managedCheckoutRepositoryName(path: string): string | undefined {
  const managed = managedCheckoutParts(path);
  return managed ? projectName(managed[1]) : undefined;
}

export const EMPTY_PROJECT_SIDEBAR_PREFERENCES: ProjectSidebarPreferences = {
  pinned: [],
  aliases: {},
};

export function loadProjectSidebarPreferences(
  storage: Pick<Storage, "getItem"> | undefined =
    typeof localStorage === "undefined" ? undefined : localStorage,
  scope?: string | null,
): ProjectSidebarPreferences {
  const key = scopedStorageKey(scope);
  if (!storage || !key) return EMPTY_PROJECT_SIDEBAR_PREFERENCES;
  try {
    const raw = storage.getItem(key);
    if (!raw) return EMPTY_PROJECT_SIDEBAR_PREFERENCES;
    const parsed = JSON.parse(raw) as Partial<ProjectSidebarPreferences>;
    return {
      pinned: Array.isArray(parsed.pinned)
        ? parsed.pinned.filter((key): key is string => typeof key === "string")
        : [],
      aliases:
        parsed.aliases && typeof parsed.aliases === "object"
          ? Object.fromEntries(
              Object.entries(parsed.aliases).filter(
                (entry): entry is [string, string] =>
                  typeof entry[1] === "string" && !!entry[1].trim(),
              ),
            )
          : {},
    };
  } catch {
    return EMPTY_PROJECT_SIDEBAR_PREFERENCES;
  }
}

export function saveProjectSidebarPreferences(
  preferences: ProjectSidebarPreferences,
  storage: Pick<Storage, "setItem"> | undefined =
    typeof localStorage === "undefined" ? undefined : localStorage,
  scope?: string | null,
): void {
  const key = scopedStorageKey(scope);
  if (!key) return;
  try {
    storage?.setItem(key, JSON.stringify(preferences));
  } catch {
    // A locked-down WebView can deny localStorage. The live UI state still works.
  }
}

export function withProjectPinned(
  preferences: ProjectSidebarPreferences,
  key: string,
  pinned: boolean,
): ProjectSidebarPreferences {
  const without = preferences.pinned.filter((candidate) => candidate !== key);
  return { ...preferences, pinned: pinned ? [...without, key] : without };
}

export function withProjectAlias(
  preferences: ProjectSidebarPreferences,
  key: string,
  alias: string,
): ProjectSidebarPreferences {
  const aliases = { ...preferences.aliases };
  const clean = alias.trim();
  if (clean) aliases[key] = clean;
  else delete aliases[key];
  return { ...preferences, aliases };
}

export function withoutProjectPreferences(
  preferences: ProjectSidebarPreferences,
  key: string,
): ProjectSidebarPreferences {
  const aliases = { ...preferences.aliases };
  delete aliases[key];
  return {
    pinned: preferences.pinned.filter((candidate) => candidate !== key),
    aliases,
  };
}

/** Group active conversations and remembered local folders into project rows.
 * Empty recent folders stay visible, which makes "Archive chats" and "Remove"
 * separate, honest actions instead of deriving the folder's existence from its
 * current chat count. */
export function groupSidebarProjects(
  conversations: ConversationMeta[],
  recentProjects: string[],
  rank: (id: string) => number,
  preferences: ProjectSidebarPreferences,
  filter = "",
): ProjectGroup[] {
  const map = new Map<string, ProjectGroup>();
  for (const conversation of conversations) {
    let key: string;
    let label: string;
    let title: string;
    let kind: ProjectGroupKind;
    if (isQuickChatProject(conversation.project, conversation.id)) {
      key = "quick-chats";
      label = "Quick chats";
      title = "Conversations in temporary the agent workspaces";
      kind = "none";
    } else if (conversation.remoteHost) {
      key = remoteProjectKey(conversation.remoteHost, conversation.project);
      label = conversation.project ? projectName(conversation.project) : conversation.remoteHost;
      title = `Remote · ${conversation.remoteHost}${conversation.project ? ` · ${conversation.project}` : ""}`;
      kind = "remote";
    } else if (conversation.project) {
      key = `p:${conversation.project}`;
      label = managedCheckoutRepositoryName(conversation.project)
        ? projectName(conversation.project)
        : projectDisplayName(conversation.project);
      title = projectDisplayTitle(conversation.project);
      kind = "local";
    } else {
      key = "none";
      label = "Other";
      title = "Conversations without a project";
      kind = "none";
    }
    let group = map.get(key);
    if (!group) {
      group = {
        key,
        label,
        title,
        kind,
        path: kind === "local" ? conversation.project : undefined,
        repositoryLabel:
          kind === "local" && conversation.project
            ? managedCheckoutRepositoryName(conversation.project)
            : undefined,
        remoteHost: kind === "remote" ? conversation.remoteHost : undefined,
        remoteRoot: kind === "remote" ? conversation.project : undefined,
        convos: [],
        latest: Infinity,
      };
      map.set(key, group);
    }
    group.convos.push(conversation);
    group.latest = Math.min(group.latest, rank(conversation.id));
  }

  recentProjects.forEach((path, index) => {
    const key = `p:${path}`;
    if (!map.has(key)) {
      map.set(key, {
        key,
        label: managedCheckoutRepositoryName(path) ? projectName(path) : projectDisplayName(path),
        title: projectDisplayTitle(path),
        kind: "local",
        path,
        repositoryLabel: managedCheckoutRepositoryName(path),
        convos: [],
        latest: conversations.length + index,
      });
    }
  });

  const query = filter.trim().toLocaleLowerCase();
  const pinnedRank = new Map(preferences.pinned.map((key, index) => [key, index]));
  return [...map.values()]
    .map((group) => ({
      ...group,
      label: preferences.aliases[group.key] ?? group.label,
      convos: [...group.convos].sort((a, b) => rank(a.id) - rank(b.id)),
    }))
    .filter((group) => {
      if (!query || group.convos.length > 0) return true;
      return `${group.label} ${group.title}`.toLocaleLowerCase().includes(query);
    })
    .sort((a, b) => {
      const aPinned = pinnedRank.get(a.key);
      const bPinned = pinnedRank.get(b.key);
      if (aPinned != null || bPinned != null) {
        if (aPinned == null) return 1;
        if (bPinned == null) return -1;
        return aPinned - bPinned;
      }
      return a.latest - b.latest;
    });
}
