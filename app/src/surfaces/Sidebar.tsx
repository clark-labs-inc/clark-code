import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { FolderPlus } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { liveSessions } from "../store/sessionStore.runtime";
import { removeRecentProject } from "../lib/localAgent";
import { codeKeyAccountBinding } from "../lib/account";
import { useSidebarDrawer } from "./sidebar/useSidebarDrawer";
import { MIN_SIDEBAR_WIDTH } from "../lib/sidebarWidth";
import { useProjectSidebarPreferences } from "./sidebar/useProjectSidebarPreferences";
import { useSidebarResize } from "./sidebar/useSidebarResize";
import { fuzzyFilter } from "../lib/fuzzy";
import { stableProjectOrder, stableRankMap } from "../lib/stableOrder";
import { cn } from "../lib/cn";
import { loadSshHosts, saveSshHosts } from "../lib/sshHosts";
import {
  groupSidebarProjects,
  sidebarConversationSearchText,
  withPinnedProjectMoved,
  withProjectRemoved,
  remoteProjectKey,
  type ProjectGroup,
} from "../lib/projectSidebar";
import {
  adjacentConversationId,
  conversationMutationStatusLabel,
  conversationRangeIds,
  type ConversationSelectionIntent,
} from "../lib/sidebarConversationInteractions";
import {
  DUR,
  EXPAND,
  EXPAND_REDUCED,
  RISE_SMALL,
  accessibleMotion,
  staggeredTransition,
} from "../lib/motion";
import { ProfileMenu } from "./ProfileMenu";
import { ProjectHeader } from "./ProjectHeader";
import { SidebarProjectMenu, type SidebarProjectMenuState } from "./sidebar/SidebarProjectMenu";
import { ProjectDragAndDrop, type ProjectDropEdge } from "./ProjectDragAndDrop";
import { ConversationRow } from "./sidebar/ConversationRow";
import { SidebarArchive } from "./sidebar/SidebarArchive";
import { sidebarProjectHost } from "../lib/sidebarProjectTarget";
import { newConversation } from "./sidebar/newSession";
import { useSpecialistStore } from "../store/specialistStore";
import { SpecialistNavigation } from "./specialists/SpecialistNavigation";
import { ConversationContextMenu } from "./sidebar/ConversationContextMenu";
import { SidebarSelectionToolbar } from "./sidebar/SidebarSelectionToolbar";
import { SidebarHeader } from "./sidebar/SidebarHeader";
import { announce } from "@atlaskit/pragmatic-drag-and-drop-live-region";

interface SidebarScrollAnchor {
  id: string;
  offset: number;
  order: string[];
  index: number;
}


export function Sidebar() {
  const collapsed = useSessionStore((s) => s.sidebarCollapsed);
  const auth = useSessionStore((s) => s.auth);
  const accountScope = codeKeyAccountBinding(auth);
  const setCollapsed = useSessionStore((s) => s.setSidebarCollapsed);
  const conversations = useSessionStore((s) => s.conversations);
  const conversationsLoading = useSessionStore((s) => s.conversationsLoading);
  const session = useSessionStore((s) => s.session);
  const activeProjectRoot = useSessionStore((s) => s.activeProjectRoot);
  const openingId = useSessionStore((s) => s.opening?.id ?? null);
  const unavailableId = useSessionStore((s) => s.unavailableConversation?.id ?? null);
  const navigatedConversationId = openingId ?? unavailableId ?? session?.id ?? null;
  // Any number of conversations can be streaming at once — each busy one gets
  // its own pulsing "Working…" dot, whether or not it's on screen.
  const runningIds = useSessionStore((s) => s.runningIds);
  // Runs that finished in the background and haven't been opened yet — the
  // blue "finished, not yet visited" dots.
  const unseenWorkIds = useSessionStore((s) => s.unseenWorkIds);
  const defaultProject = useSessionStore((s) => s.localSettings.cwd);
  const flashNotice = useSessionStore((s) => s.flashNotice);
  const selectProvider = useSessionStore((s) => s.selectProvider);
  const setProjectMode = useSessionStore((s) => s.setProjectMode);
  const setSelectedHostId = useSessionStore((s) => s.setSelectedHostId);
  const setProjectFolder = useSessionStore((s) => s.setProjectFolder);
  const setSshOpen = useSessionStore((s) => s.setSshOpen);
  const setLocalSettings = useSessionStore((s) => s.setLocalSettings);
  const recentProjects = useSessionStore((s) => s.recentProjects);
  const openConversation = useSessionStore((s) => s.openConversation);
  const restoreConversation = useSessionStore((s) => s.restoreConversation);
  const deleteConversation = useSessionStore((s) => s.deleteConversation);
  const selectedIds = useSessionStore((s) => s.selectedConversationIds);
  const mutatingIds = useSessionStore((s) => s.mutatingConversationIds);
  const conversationMutation = useSessionStore((s) => s.conversationMutation);
  const setSelection = useSessionStore((s) => s.setConversationSelection);
  const archiveSelected = useSessionStore((s) => s.archiveSelectedConversations);
  const deleteSelected = useSessionStore((s) => s.deleteSelectedConversations);
  const [renameId, setRenameId] = useState<string | null>(null);
  const finishRename = useCallback(() => setRenameId(null), []);
  const [filter, setFilter] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const [collapsedProjectKeys, setCollapsedProjectKeys] = useState<Set<string>>(
    () => new Set(),
  );
  const [archivedOpen, setArchivedOpen] = useState(false);
  const [deleteConfirming, setDeleteConfirming] = useState(false);
  const { projectPreferences, commitProjectPreferences, revealProject } = useProjectSidebarPreferences(accountScope, defaultProject, conversations, navigatedConversationId);
  const [projectMenu, setProjectMenu] = useState<SidebarProjectMenuState | null>(null);
  // The right-click menu: positioned at the cursor; acts on the selection when
  // the right-clicked row is in it, else just that row.
  const [menu, setMenu] = useState<{ x: number; y: number; ids: string[] } | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const conversationListRef = useRef<HTMLDivElement>(null);
  const scrollAnchorRef = useRef<SidebarScrollAnchor | null>(null);
  const previousConversationSignatureRef = useRef("");
  const selectionAnchorRef = useRef<string | null>(null);
  const restoredFocusIdRef = useRef<string | null>(null);
  const mutationFocusIdRef = useRef<string | null>(null);
  const mutationFocusOrderRef = useRef<string[]>([]);
  const handledMutationFocusRef = useRef<number | null>(null);
  const deleteConfirmRef = useRef<HTMLButtonElement>(null);
  const reduceMotion = useReducedMotion();
  const drawer = useSidebarDrawer(navigatedConversationId);
  const asideRef = drawer.ref;
  const { renderedWidth, maxSidebarWidth, resizingSidebar, handleSidebarResizeStart, handleSidebarResizeKey, resetSidebarWidth } = useSidebarResize(asideRef);
  const narrow = drawer.narrow;
  const closeDrawer = drawer.setOpen;

  useEffect(() => {
    setProjectMenu(null);
  }, [accountScope]);

  // No artificial cap here: a hard limit combined with ranking by recency is
  // what made the WHOLE list reshuffle once you passed it (any update landed in
  // the top-N and triggered a full re-sort). Keep every conversation; order is
  // stabilized below, and search narrows it when you actually filter.
  const visible = useMemo(
    () =>
      fuzzyFilter(
        conversations,
        filter,
        (c) => sidebarConversationSearchText(c, projectPreferences),
        5000,
      ).map((m) => m.item),
    [conversations, filter, projectPreferences.aliases],
  );
  // Rank by immutable creation time, shared by the group + row ordering: the
  // newest-created chat stays on top and activity never reshuffles the list.
  const rank = useMemo(() => {
    const m = stableRankMap(visible);
    return (id: string) => m.get(id) ?? 0;
  }, [visible]);
  // Archived conversations are hidden from the project groups and collected into
  // their own collapsed section (search still matches across both).
  const activeConvos = useMemo(
    () => visible.filter((c) => !c.archived && !c.specialist),
    [visible],
  );
  const archivedConvos = useMemo(
    () => visible.filter((c) => c.archived).sort((a, b) => b.updatedAt - a.updatedAt),
    [visible],
  );
  const rememberedProjects = useMemo(
    () => defaultProject.trim()
      ? [defaultProject.trim(), ...recentProjects.filter((path) => path !== defaultProject.trim())]
      : recentProjects,
    [defaultProject, recentProjects],
  );
  const groups = useMemo(
    () => stableProjectOrder(
      groupSidebarProjects(activeConvos, rememberedProjects, rank, projectPreferences, filter),
      (group) => {
        const pinnedIndex = projectPreferences.pinned.indexOf(group.key);
        return pinnedIndex < 0 ? projectPreferences.pinned.length : pinnedIndex;
      },
    ),
    [activeConvos, rememberedProjects, rank, projectPreferences, filter],
  );
  const pinnedGroups = useMemo(
    () => groups.filter((group) => projectPreferences.pinned.includes(group.key)),
    [groups, projectPreferences.pinned],
  );
  const activeGroupKey = useMemo(
    () => groups.find((group) =>
      group.convos.some((conversation) => conversation.id === navigatedConversationId)
    )?.key ?? null,
    [groups, navigatedConversationId],
  );

  // Folder rows begin expanded, like a file list. Navigation reveals a folder
  // the user previously collapsed.
  useEffect(() => {
    if (!activeGroupKey) return;
    setCollapsedProjectKeys((current) => {
      if (!current.has(activeGroupKey)) return current;
      const next = new Set(current); next.delete(activeGroupKey); return next;
    });
  }, [activeGroupKey]);

  const toggleProjectExpanded = useCallback((key: string) => {
    setCollapsedProjectKeys((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);
  const activeWorktreePaths = useMemo(() => {
    const activePaths = conversations
      .filter((conversation) => runningIds.includes(conversation.id))
      .map((conversation) => conversation.project)
      .filter((path): path is string => Boolean(path?.trim()));
    // An idle chat remains live in the host session pool and can reopen without
    // reconnecting. Its checkout must be protected just like a streaming one.
    for (const live of liveSessions.values()) {
      if (live.projectRoot?.trim()) activePaths.push(live.projectRoot.trim());
    }
    if (activeProjectRoot?.trim()) activePaths.push(activeProjectRoot.trim());
    return [...new Set(activePaths)];
  }, [activeProjectRoot, conversations, runningIds]);
  // Range selection follows exactly what is rendered: project grouping and
  // search filters are part of the user's visible order, so neither can make
  // Shift-click jump through invisible conversations.
  const activeConversationIds = useMemo(
    () => groups.flatMap((group) =>
      filter || !collapsedProjectKeys.has(group.key)
        ? group.convos.map((conversation) => conversation.id)
        : []
    ),
    [collapsedProjectKeys, filter, groups],
  );
  const activeConversationSignature = activeConversationIds.join("\u0000");

  // Reading every row's geometry during a scroll forces synchronous layout on
  // large histories. We only need an anchor immediately before an operation
  // changes the list, so capture it at that boundary instead.
  // The row callbacks below are passed to memoized rows, so their identity has
  // to survive a re-render or the memo never holds — and Motion re-measures
  // every layout-animated row it re-renders. Reading the volatile values from
  // a ref updated each render gives the handlers today's values without
  // putting those values in a dependency array. Behaviour is unchanged: a
  // handler still sees whatever was current when it was called.
  const latest = useRef({ mutatingIds, selectedIds, activeConversationIds });
  // Committed in a layout effect rather than during render: a render React
  // discards must not leave its values behind. This still runs before the
  // browser paints, so no event handler can observe a stale value.
  useLayoutEffect(() => {
    latest.current = { mutatingIds, selectedIds, activeConversationIds };
  });

  const captureScrollAnchor = useCallback(() => {
    const { activeConversationIds } = latest.current;
    const container = conversationListRef.current;
    if (!container) return;
    const containerRect = container.getBoundingClientRect();
    const rows = Array.from(
      container.querySelectorAll<HTMLElement>("[data-sidebar-conversation-id]"),
    );
    const firstVisible = rows.find((row) => row.getBoundingClientRect().bottom > containerRect.top + 1);
    const id = firstVisible?.dataset.sidebarConversationId;
    const index = id ? activeConversationIds.indexOf(id) : -1;
    if (!firstVisible || !id || index < 0) return;
    scrollAnchorRef.current = {
      id,
      index,
      order: activeConversationIds,
      offset: firstVisible.getBoundingClientRect().top - containerRect.top,
    };
  }, []);

  const focusConversation = useCallback((id: string): boolean => {
    const container = conversationListRef.current;
    if (!container) return false;
    const button = Array.from(
      container.querySelectorAll<HTMLButtonElement>("[data-sidebar-conversation-button]"),
    ).find((candidate) => candidate.dataset.sidebarConversationButton === id);
    if (!button) return false;
    button.focus({ preventScroll: true });
    button.scrollIntoView({ block: "nearest" });
    return true;
  }, []);

  const requestMutationFocus = useCallback((id: string | null) => {
    mutationFocusIdRef.current = id;
    mutationFocusOrderRef.current = latest.current.activeConversationIds;
  }, []);

  // Keep the first visible conversation at the same visual offset when rows
  // above it archive, delete, or restore. Motion smooths the surrounding move;
  // this preserves the user's reading position underneath it.
  useLayoutEffect(() => {
    const previous = previousConversationSignatureRef.current;
    if (previous === activeConversationSignature) return;
    const container = conversationListRef.current;
    const anchor = scrollAnchorRef.current;
    if (container && anchor) {
      const rows = Array.from(
        container.querySelectorAll<HTMLElement>("[data-sidebar-conversation-id]"),
      );
      const currentIds = new Set(rows.map((row) => row.dataset.sidebarConversationId).filter(Boolean));
      const fallbackId = [
        anchor.id,
        ...anchor.order.slice(anchor.index + 1),
        ...anchor.order.slice(0, anchor.index).reverse(),
      ].find((id) => currentIds.has(id));
      const target = rows.find((row) => row.dataset.sidebarConversationId === fallbackId);
      if (target) {
        const delta = target.getBoundingClientRect().top - container.getBoundingClientRect().top - anchor.offset;
        if (Math.abs(delta) > 0.5) container.scrollTop += delta;
      }
    }
    previousConversationSignatureRef.current = activeConversationSignature;
  }, [activeConversationSignature]);

  useEffect(() => {
    const restoredId = restoredFocusIdRef.current;
    if (!restoredId || !activeConversationIds.includes(restoredId)) return;
    const focus = () => {
      focusConversation(restoredId);
      restoredFocusIdRef.current = null;
    };
    if (typeof requestAnimationFrame === "function") {
      const frame = requestAnimationFrame(focus);
      return () => cancelAnimationFrame(frame);
    }
    focus();
  }, [activeConversationIds, activeConversationSignature]);

  // A mutation removes the focused row or the selection toolbar that launched
  // it. Keep keyboard and screen-reader users oriented by moving to the next
  // surviving visible conversation, then to the labeled list if none survives.
  useEffect(() => {
    if (
      !conversationMutation ||
      conversationMutation.pending > 0 ||
      conversationMutation.kind === "restore" ||
      handledMutationFocusRef.current === conversationMutation.id
    ) {
      return;
    }
    handledMutationFocusRef.current = conversationMutation.id;
    const requestedId = mutationFocusIdRef.current;
    const activeIds = new Set(activeConversationIds);
    const anchorOrder = mutationFocusOrderRef.current.length
      ? mutationFocusOrderRef.current
      : scrollAnchorRef.current?.order ?? activeConversationIds;
    const requestedIndex = requestedId ? anchorOrder.indexOf(requestedId) : -1;
    const candidates = requestedId
      ? [
          requestedId,
          ...anchorOrder.slice(requestedIndex + 1),
          ...anchorOrder.slice(0, Math.max(0, requestedIndex)).reverse(),
          ...activeConversationIds,
        ]
      : [];
    const nextId = candidates.find((id): id is string => Boolean(id && activeIds.has(id)));
    if (!nextId || !focusConversation(nextId)) conversationListRef.current?.focus({ preventScroll: true });
    mutationFocusIdRef.current = null;
    mutationFocusOrderRef.current = [];
  }, [activeConversationIds, activeConversationSignature, conversationMutation]);

  useEffect(() => {
    if (selectedIds.size === 0 || mutatingIds.size > 0) setDeleteConfirming(false);
  }, [mutatingIds.size, selectedIds.size]);

  useEffect(() => {
    if (deleteConfirming) deleteConfirmRef.current?.focus();
  }, [deleteConfirming]);

  const selectConversation = useCallback((
    id: string,
    intent: ConversationSelectionIntent,
    additive = false,
  ) => {
    const { mutatingIds, selectedIds, activeConversationIds } = latest.current;
    if (mutatingIds.has(id)) return;
    if (intent === "open") {
      closeDrawer(false);
      selectionAnchorRef.current = id;
      setSelection(new Set());
      useSpecialistStore.getState().close();
      const conversation = useSessionStore.getState().conversations.find((c) => c.id === id);
      const key = conversation?.remoteHost ? remoteProjectKey(conversation.remoteHost, conversation.project)
        : conversation?.project ? "p:" + conversation.project : null;
      revealProject(key);
      void openConversation(id);
      return;
    }
    if (intent === "toggle") {
      selectionAnchorRef.current = id;
      const next = new Set(selectedIds);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      setSelection(next);
      return;
    }
    const anchor = selectionAnchorRef.current;
    const range = conversationRangeIds(activeConversationIds, anchor, id);
    if (range.length === 0) return;
    if (!anchor || !activeConversationIds.includes(anchor)) selectionAnchorRef.current = id;
    setSelection(additive ? new Set([...selectedIds, ...range]) : new Set(range));
  }, [closeDrawer, openConversation, revealProject, setSelection]);

  const extendSelectionWithKeyboard = useCallback((id: string, direction: -1 | 1) => {
    const { activeConversationIds } = latest.current;
    if (!selectionAnchorRef.current || !activeConversationIds.includes(selectionAnchorRef.current)) {
      selectionAnchorRef.current = id;
    }
    const target = adjacentConversationId(activeConversationIds, id, direction);
    if (!target) return;
    selectConversation(target, "range");
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(() => focusConversation(target));
    else focusConversation(target);
  }, [focusConversation, selectConversation]);

  const restoreAndOpenConversation = useCallback((id: string) => {
    captureScrollAnchor();
    restoredFocusIdRef.current = id;
    void restoreConversation(id);
  }, [captureScrollAnchor, restoreConversation]);

  const deleteArchivedConversation = useCallback((id: string) => {
    captureScrollAnchor();
    requestMutationFocus(null);
    void deleteConversation(id);
  }, [captureScrollAnchor, deleteConversation, requestMutationFocus]);
  const archiveSelectedWithFocus = () => {
    captureScrollAnchor();
    requestMutationFocus(selectionAnchorRef.current);
    void archiveSelected();
  };
  const deleteSelectedWithFocus = () => {
    captureScrollAnchor();
    requestMutationFocus(selectionAnchorRef.current);
    void deleteSelected();
  };
  const mutationInFlight = mutatingIds.size > 0;
  const selectionStatus = conversationMutation
    ? conversationMutationStatusLabel(conversationMutation)
    : selectedIds.size > 0
      ? `${selectedIds.size} ${selectedIds.size === 1 ? "conversation" : "conversations"} selected.`
      : "";

  const removeProject = (group: ProjectGroup) => {
    if (group.path) {
      if (defaultProject.trim() === group.path) setLocalSettings({ cwd: "" });
      useSessionStore.setState({ recentProjects: removeRecentProject(group.path, accountScope) });
    }
    commitProjectPreferences(withProjectRemoved(projectPreferences, group.key, true));
    setProjectMenu(null);
    setFilter("");
    setSearchOpen(false);
    setSelection(new Set());
    flashNotice("Removed " + group.label + " from the sidebar. Its chats are still in search.");
    requestAnimationFrame(() => conversationListRef.current?.focus());
  };

  const movePinnedProjectTo = useCallback((key: string, index: number, label: string) => {
    const next = withPinnedProjectMoved(projectPreferences, key, index);
    if (next === projectPreferences) return;
    commitProjectPreferences(next);
    announce(`Project ${label} moved to position ${index + 1} of ${next.pinned.length}.`);
  }, [commitProjectPreferences, projectPreferences]);

  const dropPinnedProject = useCallback((
    sourceKey: string,
    targetKey: string,
    edge: ProjectDropEdge,
  ) => {
    const remaining = projectPreferences.pinned.filter((key) => key !== sourceKey);
    const targetIndex = remaining.indexOf(targetKey);
    if (targetIndex < 0) return;
    const destinationIndex = targetIndex + (edge === "bottom" ? 1 : 0);
    const label = groups.find((group) => group.key === sourceKey)?.label ?? "Project";
    movePinnedProjectTo(sourceKey, destinationIndex, label);
  }, [groups, movePinnedProjectTo, projectPreferences.pinned]);

  const openProjectMenu = (group: ProjectGroup, button: HTMLButtonElement) => {
    if (projectMenu?.group.key === group.key) {
      setProjectMenu(null);
      return;
    }
    setMenu(null);
    const rect = button.getBoundingClientRect();
    const width = 240;
    const estimatedHeight = 244;
    const left = Math.max(8, Math.min(rect.right + 6, window.innerWidth - width - 8));
    const top = Math.max(8, Math.min(rect.top - 8, window.innerHeight - estimatedHeight - 8));
    setProjectMenu({ group, position: { left, top }, trigger: button });
  };

  const archiveProjectChats = (group: ProjectGroup) => {
    if (group.convos.length === 0) return;
    selectionAnchorRef.current = group.convos[0]?.id ?? null;
    setSelection(new Set(group.convos.map((conversation) => conversation.id)));
    archiveSelectedWithFocus();
  };

  const startProjectSession = (group: ProjectGroup) => {
    setProjectMenu(null);
    if (group.kind === "remote") {
      const hosts = loadSshHosts(accountScope);
      const host = sidebarProjectHost(group, hosts);
      if (!host) {
        flashNotice(`Reconnect ${group.label} in Remote hosts to start a new session.`);
        setSshOpen(true);
        return;
      }
      saveSshHosts(hosts.map((candidate) => candidate.id === host.id ? host : candidate), accountScope);
      useSessionStore.getState().bumpSshHostsRevision();
      selectProvider("local");
      setProjectMode("remote");
      setSelectedHostId(host.id);
      newConversation(group.label);
      return;
    }
    if (group.path) {
      selectProvider("local");
      setProjectMode("local");
      setProjectFolder(group.path);
      newConversation(group.label);
    }
  };

  const openContextMenu = useCallback((e: React.MouseEvent, id: string) => {
    const { mutatingIds, selectedIds } = latest.current;
    e.preventDefault();
    if (mutatingIds.size > 0) return;
    setProjectMenu(null);
    const rect = e.currentTarget.getBoundingClientRect();
    const x = Math.max(8, Math.min(e.type === "click" ? rect.right : e.clientX, window.innerWidth - 216));
    const y = Math.max(8, Math.min(e.type === "click" ? rect.bottom : e.clientY, window.innerHeight - 180));
    // Act on the whole selection when the right-clicked row is part of it;
    // otherwise the action targets just this row (and the selection becomes it,
    // so the visual matches what the menu will act on).
    if (selectedIds.has(id)) {
      selectionAnchorRef.current = id;
      setMenu({ x, y, ids: [...selectedIds] });
    } else {
      selectionAnchorRef.current = id;
      setSelection(new Set());
      setMenu({ x, y, ids: [id] });
    }
  }, [setSelection]);

  const archiveSection = <SidebarArchive archivedConvos={archivedConvos} open={archivedOpen || Boolean(filter.trim())}
    onToggle={() => setArchivedOpen((open) => !open)} mutatingIds={mutatingIds}
    mutationKind={conversationMutation?.kind} onRestore={restoreAndOpenConversation} onDelete={deleteArchivedConversation} />;

  if (narrow ? !drawer.open : collapsed) {
    return (
      <div className="flex w-12 shrink-0 flex-col items-center gap-1 bg-bg-secondary py-2">
        <SidebarHeader rail onToggle={() => narrow ? drawer.setOpen(true) : setCollapsed(false)} filter={filter} onFilter={setFilter}
          searchRef={searchInputRef} searchOpen={searchOpen} onSearchOpenChange={setSearchOpen} />
        <SpecialistNavigation rail />
        <div className="mt-auto">
          <ProfileMenu variant="rail" />
        </div>
      </div>
    );
  }

  return (
    <>
    {narrow && <button type="button" tabIndex={-1} aria-label="Close sidebar" onClick={() => drawer.setOpen(false)} className="fixed inset-0 z-30 bg-scrim" />}
    <aside
      role={narrow ? "dialog" : undefined}
      aria-modal={narrow || undefined}
      aria-label="Sidebar navigation"
      tabIndex={narrow ? -1 : undefined}
      onKeyDown={drawer.onKeyDown}
      ref={asideRef}
      className={cn(
        "flex min-h-0 shrink-0 overflow-hidden text-base leading-5",
        resizingSidebar && "cursor-col-resize select-none",
        narrow && "fixed inset-y-0 left-0 z-40 shadow-lifted",
      )}
      style={{ width: narrow ? "min(320px, calc(100vw - 40px))" : renderedWidth }}
    >
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-bg-secondary">
      <SidebarHeader rail={false} onToggle={() => narrow ? drawer.setOpen(false) : setCollapsed(true)} filter={filter} onFilter={setFilter}
        searchRef={searchInputRef} searchOpen={searchOpen} onSearchOpenChange={setSearchOpen} />

      <p id="sidebar-selection-help" className="sr-only">
        Use Shift-click or Shift+Arrow Up and Down to select a range. Command or Control-click toggles one conversation. Press Escape to clear the selection.
      </p>
      <p className="sr-only" role="status" aria-live="polite" aria-atomic="true">
        {selectionStatus}
      </p>

      <div
        ref={conversationListRef}
        aria-label="Conversations"
        tabIndex={-1}
        className="min-h-0 flex-1 overflow-y-auto px-2 pb-4"
        onClick={(e) => {
          // A plain click on empty list space (not on a row) clears the
          // Shift-click selection.
          if (e.target === e.currentTarget && selectedIds.size > 0) setSelection(new Set());
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape" && selectedIds.size > 0) {
            e.preventDefault();
            setSelection(new Set());
          }
        }}
      >
        <div className="group mb-1 flex h-8 items-center px-2">
          <h2 className="flex-1 text-xs font-medium text-ink-muted">Projects</h2>
          <button type="button" aria-label="Add project" title="Add project — choose a folder or remote host"
            onClick={() => useSessionStore.getState().setNewProjectOpen(true)}
            className="grid size-7 place-items-center rounded-md text-ink-muted hover:bg-bg-hover hover:text-ink">
            <FolderPlus className="size-4" />
          </button>
        </div>

        {conversations.length === 0 && groups.length === 0 ? (
          <p className="px-1 py-6 text-center text-sm text-ink-faint">
            {conversationsLoading ? "Loading conversations…" : "Add a project folder, or choose New chat to get started."}
          </p>
        ) : visible.length === 0 && groups.length === 0 ? (
          <p className="px-1 py-6 text-center text-sm text-ink-faint">
            No projects or chats match “{filter}”.
          </p>
        ) : (
          <div className="flex flex-col">
            <AnimatePresence initial={false} mode="popLayout">
              {groups.map((g) => {
                const expanded = Boolean(filter) || !collapsedProjectKeys.has(g.key);
                const conversationPanelId = `project-conversations-${encodeURIComponent(g.key)}`;
                return (
                  <ProjectDragAndDrop
                    key={g.key}
                    projectKey={g.key}
                    label={g.label}
                    enabled={!filter && pinnedGroups.length > 1 && projectPreferences.pinned.includes(g.key)}
                    onDropProject={dropPinnedProject}
                  >
                    {(dragHandleRef) => (
                      <m.section
                        layout={reduceMotion ? false : "position"}
                        {...accessibleMotion(RISE_SMALL, reduceMotion)}
                        transition={staggeredTransition(reduceMotion, 0, 0.04, { duration: DUR.fast })}
                      >
                        <ProjectHeader
                          group={g}
                          expanded={expanded}
                          conversationPanelId={conversationPanelId}
                          menuOpen={projectMenu?.group.key === g.key}
                          reorderable={!filter && pinnedGroups.length > 1 && projectPreferences.pinned.includes(g.key)}
                          dragHandleRef={dragHandleRef}
                          onToggle={() => toggleProjectExpanded(g.key)}
                          onOpenMenu={(button) => openProjectMenu(g, button)}
                          onNewSession={() => startProjectSession(g)}
                        />
                        <AnimatePresence initial={false}>
                          {expanded && (
                            <m.div
                              id={conversationPanelId}
                              {...(reduceMotion ? EXPAND_REDUCED : EXPAND)}
                              className="overflow-hidden pl-6"
                            >
                              {g.convos.length === 0 && <div className="px-2 py-2 text-sm text-ink-muted">
                                <button type="button" onClick={() => startProjectSession(g)} className="text-ink-muted hover:text-ink">Start a chat</button>
                              </div>}
                              <div className="flex flex-col py-0.5">
                                {g.convos.map((c) => {
                                  const mutation = mutatingIds.has(c.id)
                                    ? conversationMutation?.kind ?? "archive"
                                    : null;
                                  return (
                                    <ConversationRow
                                      key={c.id}
                                      c={c}
                                      renameRequested={renameId === c.id}
                                      onRenameDone={finishRename}
                                      active={navigatedConversationId === c.id}
                                      streaming={runningIds.includes(c.id)}
                                      unseen={
                                        unseenWorkIds.includes(c.id) &&
                                        navigatedConversationId !== c.id
                                      }
                                      opening={openingId === c.id}
                                      selected={selectedIds.has(c.id)}
                                      mutation={mutation}
                                      onSelect={selectConversation}
                                      onRangeStep={extendSelectionWithKeyboard}
                                      onContextMenu={openContextMenu}
                                    />
                                  );
                                })}
                              </div>
                            </m.div>
                          )}
                        </AnimatePresence>
                      </m.section>
                    )}
                  </ProjectDragAndDrop>
                );
              })}
            </AnimatePresence>

            {groups.length === 0 && archivedConvos.length > 0 && (
              <p className="px-1 py-6 text-center text-sm text-ink-faint">
                {filter ? "No active chats match. Archived matches are below." : "No active conversations."}
              </p>
            )}
          </div>
        )}
        {filter && archiveSection}
        <SpecialistNavigation />
      </div>

      {!filter && archiveSection}

      <div className="relative shrink-0">
        <SidebarSelectionToolbar selectedCount={selectedIds.size} hasMutation={Boolean(conversationMutation)}
          reduceMotion={reduceMotion} deleteConfirming={deleteConfirming} deleteConfirmRef={deleteConfirmRef}
          selectionStatus={selectionStatus} mutationInFlight={mutationInFlight} setDeleteConfirming={setDeleteConfirming}
          deleteSelectedWithFocus={deleteSelectedWithFocus} archiveSelectedWithFocus={archiveSelectedWithFocus}
          clearSelection={() => setSelection(new Set())} />
        <div className="px-2 py-1">
          <ProfileMenu variant="sidebar" />
        </div>
      </div>

      {menu && (
        <ConversationContextMenu
          menu={{ x: menu.x, y: menu.y }}
          count={menu.ids.length}
          onClose={() => {
            document.querySelector<HTMLButtonElement>(`[data-sidebar-conversation-button="${CSS.escape(menu.ids[0])}"]`)?.focus();
            setMenu(null);
          }}
          onRename={() => setRenameId(menu.ids[0])}
          onArchive={() => {
              setSelection(new Set(menu.ids));
              requestMutationFocus(selectionAnchorRef.current);
              void archiveSelected();
          }}
          onDelete={() => {
              setSelection(new Set(menu.ids));
              requestMutationFocus(selectionAnchorRef.current);
              void deleteSelected();
          }}
        />
      )}

      {projectMenu && <SidebarProjectMenu projectMenu={projectMenu} projectPreferences={projectPreferences}
        pinnedGroups={pinnedGroups} activeWorktreePaths={activeWorktreePaths}
        onClose={() => {
          const trigger = projectMenu.trigger;
          setProjectMenu(null);
          requestAnimationFrame(() => { if (trigger.isConnected) trigger.focus(); });
        }}
        commitProjectPreferences={commitProjectPreferences} movePinnedProjectTo={movePinnedProjectTo}
        archiveProjectChats={archiveProjectChats} onRemove={removeProject} />}

      </div>
      {!narrow && <div
        role="separator"
        aria-label="Resize sidebar"
        aria-orientation="vertical"
        aria-valuemin={MIN_SIDEBAR_WIDTH}
        aria-valuemax={Math.max(MIN_SIDEBAR_WIDTH, maxSidebarWidth)}
        aria-valuenow={renderedWidth}
        tabIndex={0}
        title="Drag to resize sidebar · Double-click to reset"
        onDoubleClick={resetSidebarWidth}
        onKeyDown={handleSidebarResizeKey}
        onMouseDown={handleSidebarResizeStart}
        className="group relative z-20 w-2 shrink-0 touch-none cursor-col-resize outline-none"
      >
        <span
          className={cn(
            "absolute inset-y-0 left-1/2 w-px -translate-x-1/2 transition-colors",
            resizingSidebar
              ? "bg-accent"
              : "bg-border-subtle group-hover:bg-accent/70 group-focus-visible:bg-accent",
          )}
        />
      </div>}
    </aside>
    </>
  );
}
