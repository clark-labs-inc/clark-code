import { useSessionStore } from "../../store/sessionStore";
import { openRemote } from "../../store/sessionStore.runtime";
import { projectName } from "../../lib/localAgent";
import { codeKeyAccountBinding } from "../../lib/account";
import { getBridge } from "../../core-bridge/bridge";
import { openProjectPath } from "../../lib/openPath";
import { loadSshHosts, saveSshHosts } from "../../lib/sshHosts";
import { withProjectAlias, withProjectPinned, type ProjectGroup, type ProjectSidebarPreferences } from "../../lib/projectSidebar";
import { ProjectActionsMenu, type ProjectMenuPosition, type ProjectMoveDestination } from "../ProjectActionsMenu";
import { newConversation } from "./newSession";
import { useSpecialistStore } from "../../store/specialistStore";

export interface SidebarProjectMenuState {
  group: ProjectGroup;
  position: ProjectMenuPosition;
  trigger: HTMLButtonElement;
}
function projectMoveDestinations(
  pinnedGroups: ProjectGroup[],
  currentKey: string,
  pinnedKeys: string[],
): ProjectMoveDestination[] {
  const currentIndex = pinnedKeys.indexOf(currentKey);
  if (currentIndex < 0) return [];
  const remainingKeys = pinnedKeys.filter((key) => key !== currentKey);
  return [
    { index: 0, label: "First", current: currentIndex === 0 },
    ...pinnedGroups
      .filter((group) => group.key !== currentKey)
      .map((group) => {
        const index = remainingKeys.indexOf(group.key) + 1;
        return { index, label: `After ${group.label}`, current: index === currentIndex };
      }),
  ];
}


export function SidebarProjectMenu({ projectMenu, projectPreferences, pinnedGroups, activeWorktreePaths, onClose, commitProjectPreferences, movePinnedProjectTo, archiveProjectChats, onRemove }: {
  projectMenu: SidebarProjectMenuState;
  projectPreferences: ProjectSidebarPreferences;
  pinnedGroups: ProjectGroup[];
  activeWorktreePaths: string[];
  onClose: () => void;
  commitProjectPreferences: (preferences: ProjectSidebarPreferences) => void;
  movePinnedProjectTo: (key: string, index: number, label: string) => void;
  archiveProjectChats: (group: ProjectGroup) => void;
  onRemove: (group: ProjectGroup) => void;
}) {
  const auth = useSessionStore((s) => s.auth);
  const localSettings = useSessionStore((s) => s.localSettings);
  const flashNotice = useSessionStore((s) => s.flashNotice);
  const selectProvider = useSessionStore((s) => s.selectProvider);
  const setProjectMode = useSessionStore((s) => s.setProjectMode);
  const setSelectedHostId = useSessionStore((s) => s.setSelectedHostId);
  const setProjectFolder = useSessionStore((s) => s.setProjectFolder);
  const openTerminal = useSessionStore((s) => s.openProjectTerminal);
  const accountScope = codeKeyAccountBinding(auth);
  const openProjectTerminal = async (path: string) => { useSpecialistStore.getState().close(); await openTerminal(path); };
  return <ProjectActionsMenu
          key={projectMenu.group.key}
          group={projectMenu.group}
          position={projectMenu.position}
          pinned={projectPreferences.pinned.includes(projectMenu.group.key)}
          moveDestinations={projectMoveDestinations(
            pinnedGroups,
            projectMenu.group.key,
            projectPreferences.pinned,
          )}
          onClose={onClose}
          onPin={(pinned) =>
            commitProjectPreferences(
              withProjectPinned(projectPreferences, projectMenu.group.key, pinned),
            )
          }
          onMove={(destinationIndex) => {
            const { group, trigger } = projectMenu;
            movePinnedProjectTo(group.key, destinationIndex, group.label);
            requestAnimationFrame(() => {
              if (trigger.isConnected) trigger.focus();
            });
          }}
          onReveal={() => {
            const path = projectMenu.group.path;
            if (path) void openProjectPath(path, "", true);
          }}
          onCreateWorktree={async (name) => {
            const path = projectMenu.group.path;
            const bridge = await getBridge();
            if (!bridge.createPermanentWorktree) {
              throw new Error("Permanent worktrees are available in the desktop app.");
            }
            if (path) {
              const createdPath = await bridge.createPermanentWorktree(path, name);
              await openProjectTerminal(createdPath);
              flashNotice(`Created worktree ${projectName(createdPath)}`);
              return;
            }
            const group = projectMenu.group;
            const hosts = loadSshHosts(accountScope);
            const host = hosts.find((candidate) =>
              candidate.host.trim() === group.remoteHost
              && candidate.remoteRoot.trim() === group.remoteRoot
            ) ?? hosts.find((candidate) => candidate.host.trim() === group.remoteHost);
            if (!host) throw new Error(`Reconnect ${group.label} in Remote hosts first.`);
            const connection = await openRemote(
              host,
              localSettings,
              group.remoteRoot || host.remoteRoot,
            );
            const createdPath = await bridge.createPermanentWorktree(
              connection.cwd,
              name,
              { id: connection.id },
            );
            saveSshHosts(
              hosts.map((candidate) => candidate.id === host.id
                ? { ...candidate, remoteRoot: createdPath }
                : candidate),
              accountScope,
            );
            selectProvider("local");
            setProjectMode("remote");
            setSelectedHostId(host.id);
            newConversation();
            flashNotice(`Created worktree ${projectName(createdPath)}`);
          }}
          onListManagedWorktrees={async () => {
            const path = projectMenu.group.path;
            if (!path) throw new Error("A local project folder is required.");
            const bridge = await getBridge();
            if (!bridge.listManagedWorktrees) {
              throw new Error("Managed worktrees are available in the desktop app.");
            }
            return bridge.listManagedWorktrees(path);
          }}
          onUseManagedWorktree={(path) => {
            selectProvider("local");
            setProjectMode("local");
            setProjectFolder(path);
            newConversation();
            flashNotice("Using isolated checkout " + projectName(path) + " for the next chat");
          }}
          onSaveManagedWorktreeBranch={async (id) => {
            const path = projectMenu.group.path;
            if (!path) throw new Error("A local project folder is required.");
            const bridge = await getBridge();
            if (!bridge.saveManagedWorktreeBranch) {
              throw new Error("Saving managed worktree commits is available in the desktop app.");
            }
            const receipt = await bridge.saveManagedWorktreeBranch(path, id);
            flashNotice("Saved detached commits as " + receipt.branch);
            return receipt;
          }}
          onCleanupManagedWorktree={async (id) => {
            const path = projectMenu.group.path;
            if (!path) throw new Error("A local project folder is required.");
            const bridge = await getBridge();
            if (!bridge.cleanupManagedWorktree) {
              throw new Error("Managed worktrees are available in the desktop app.");
            }
            await bridge.cleanupManagedWorktree(path, id);
            flashNotice("Archived isolated checkout");
          }}
          activeWorktreePaths={activeWorktreePaths}
          onRename={(name) =>
            commitProjectPreferences(
              withProjectAlias(projectPreferences, projectMenu.group.key, name),
            )
          }
          onArchive={() => archiveProjectChats(projectMenu.group)}
          onRemove={() => onRemove(projectMenu.group)}
        />;
}
