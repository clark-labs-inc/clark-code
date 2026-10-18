import { useCallback, useEffect, useMemo, useState } from "react";
import { loadProjectSidebarPreferences, remoteProjectKey, saveProjectSidebarPreferences, withProjectRemoved, type ProjectSidebarPreferences } from "../../lib/projectSidebar";
import type { ConversationMeta } from "../../lib/history";

export function useProjectSidebarPreferences(
  accountScope: string | null,
  defaultProject: string,
  conversations: ConversationMeta[],
  navigatedConversationId: string | null,
) {
  const [projectPreferences, setProjectPreferences] = useState(
    () => loadProjectSidebarPreferences(undefined, accountScope),
  );
  useEffect(() => {
    setProjectPreferences(loadProjectSidebarPreferences(undefined, accountScope));
  }, [accountScope]);

  const commitProjectPreferences = useCallback((next: ProjectSidebarPreferences) => {
    saveProjectSidebarPreferences(next, undefined, accountScope);
    setProjectPreferences(next);
  }, [accountScope]);

  const revealProject = useCallback((key: string | null) => {
    if (!key) return;
    setProjectPreferences((current) => {
      if (!current.removed.includes(key)) return current;
      const next = withProjectRemoved(current, key, false);
      saveProjectSidebarPreferences(next, undefined, accountScope);
      return next;
    });
  }, [accountScope]);

  const navigatedKey = useMemo(() => {
    const conversation = conversations.find((c) => c.id === navigatedConversationId);
    return conversation?.remoteHost
      ? remoteProjectKey(conversation.remoteHost, conversation.project)
      : conversation?.project ? "p:" + conversation.project : null;
  }, [conversations, navigatedConversationId]);

  // Choosing a folder or opening a saved chat adds a removed project back.
  // Separate effects let removal clear the default without re-adding an active
  // chat's folder when that chat itself did not change.
  useEffect(() => { revealProject(navigatedKey); }, [navigatedKey, revealProject]);
  useEffect(() => {
    revealProject(defaultProject.trim() ? "p:" + defaultProject.trim() : null);
  }, [defaultProject, revealProject]);

  return { projectPreferences, commitProjectPreferences, revealProject };
}
