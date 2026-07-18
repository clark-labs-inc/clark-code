import type { ConversationMeta } from "./history";
import { projectName } from "./localAgent";

const STORAGE_KEY = "clark-desktop:project-sidebar";

export type ProjectGroupKind = "remote" | "local" | "none";

export interface ProjectGroup {
  key: string;
  label: string;
  title: string;
  kind: ProjectGroupKind;
  /** The local folder behind the group (local projects only). */
  path?: string;
  convos: ConversationMeta[];
  latest: number;
}

export interface ProjectSidebarPreferences {
  pinned: string[];
  aliases: Record<string, string>;
}

export const EMPTY_PROJECT_SIDEBAR_PREFERENCES: ProjectSidebarPreferences = {
  pinned: [],
  aliases: {},
};

export function loadProjectSidebarPreferences(
  storage: Pick<Storage, "getItem"> | undefined =
    typeof localStorage === "undefined" ? undefined : localStorage,
): ProjectSidebarPreferences {
  if (!storage) return EMPTY_PROJECT_SIDEBAR_PREFERENCES;
  try {
    const raw = storage.getItem(STORAGE_KEY);
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
): void {
  try {
    storage?.setItem(STORAGE_KEY, JSON.stringify(preferences));
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
    if (conversation.remoteHost) {
      key = `r:${conversation.remoteHost}`;
      label = conversation.remoteHost;
      title = `Remote · ${conversation.remoteHost}${conversation.project ? ` · ${conversation.project}` : ""}`;
      kind = "remote";
    } else if (conversation.project) {
      key = `p:${conversation.project}`;
      label = projectName(conversation.project);
      title = conversation.project;
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
        label: projectName(path),
        title: path,
        kind: "local",
        path,
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
