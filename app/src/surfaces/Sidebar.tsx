import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import {
  Archive, Trash2, X,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { liveSessions, openRemote } from "../store/sessionStore.runtime";
import { projectName, removeRecentProject } from "../lib/localAgent";
import { codeKeyAccountBinding } from "../lib/account";
import { useSidebarDrawer } from "./sidebar/useSidebarDrawer";
import {
  DEFAULT_SIDEBAR_WIDTH,
  MIN_SIDEBAR_WIDTH,
  constrainSidebarWidth,
  loadSidebarWidth,
  saveSidebarWidth,
} from "../lib/sidebarWidth";
import { fuzzyFilter } from "../lib/fuzzy";
import { stableProjectOrder, stableRankMap } from "../lib/stableOrder";
import { cn } from "../lib/cn";
import { getBridge } from "../core-bridge/bridge";
import { openProjectPath } from "../lib/openPath";
import { loadSshHosts, saveSshHosts } from "../lib/sshHosts";
import {
  groupSidebarProjects,
  sidebarConversationSearchText,
  loadProjectSidebarPreferences,
  saveProjectSidebarPreferences,
  withProjectAlias,
  withProjectPinned,
  withPinnedProjectMoved,
  withoutProjectPreferences,
  type ProjectGroup,
  type ProjectSidebarPreferences,
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
  RISE,
  RISE_SMALL,
  accessibleMotion,
  staggeredTransition,
} from "../lib/motion";
import { ProfileMenu } from "./ProfileMenu";
import {
  ProjectActionsMenu,
  ProjectHeader,
  type ProjectMoveDestination,
  type ProjectMenuPosition,
} from "./ProjectActionsMenu";
import { ProjectDragAndDrop, type ProjectDropEdge } from "./ProjectDragAndDrop";
import { ConversationRow } from "./sidebar/ConversationRow";
import { SidebarArchive } from "./sidebar/SidebarArchive";
import { sidebarProjectHost } from "../lib/sidebarProjectTarget";
import { newConversation } from "./sidebar/newSession";
import { useSpecialistStore } from "../store/specialistStore";
import { SpecialistNavigation } from "./specialists/SpecialistNavigation";
import { ConversationContextMenu } from "./sidebar/ConversationContextMenu";
import { SidebarHeader } from "./sidebar/SidebarHeader";
import { announce } from "@atlaskit/pragmatic-drag-and-drop-live-region";

interface SidebarScrollAnchor {
  id: string;
  offset: number;
  order: string[];
  index: number;
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


export function Sidebar({
  artifactCount = 0,
  onOpenArtifacts,
}: {
  artifactCount?: number;
  onOpenArtifacts?: () => void;
}) {
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
  const localSettings = useSessionStore((s) => s.localSettings);
  const flashNotice = useSessionStore((s) => s.flashNotice);
  const selectProvider = useSessionStore((s) => s.selectProvider);
  const setProjectMode = useSessionStore((s) => s.setProjectMode);
  const setSelectedHostId = useSessionStore((s) => s.setSelectedHostId);
  const setProjectFolder = useSessionStore((s) => s.setProjectFolder);
  const setSshOpen = useSessionStore((s) => s.setSshOpen);
  const openProjectTerminalAction = useSessionStore((s) => s.openProjectTerminal);
  const openProjectTerminal = async (path?: string) => {
    useSpecialistStore.getState().close();
    await openProjectTerminalAction(path);
  };
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
  const [expandedProjectKeys, setExpandedProjectKeys] = useState<Set<string>>(
    () => new Set(["quick-chats"]),
  );
  const [archivedOpen, setArchivedOpen] = useState(false);
  useEffect(() => { if (filter.trim()) setArchivedOpen(true); }, [filter]);
  const [deleteConfirming, setDeleteConfirming] = useState(false);
  const [projectPreferences, setProjectPreferences] = useState<ProjectSidebarPreferences>(
    () => loadProjectSidebarPreferences(undefined, accountScope),
  );
  const [projectMenu, setProjectMenu] = useState<{
    group: ProjectGroup;
    position: ProjectMenuPosition;
    trigger: HTMLButtonElement;
  } | null>(null);
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
  // The conversation sidebar is horizontally resizable; the width persists per
  // window and is clamped so the conversation pane keeps a usable minimum.
  const [sidebarWidth, setSidebarWidth] = useState(() => loadSidebarWidth());
  const [viewportWidth, setViewportWidth] = useState(() => window.innerWidth);
  const [resizingSidebar, setResizingSidebar] = useState(false);
  const drawer = useSidebarDrawer(navigatedConversationId);
  const asideRef = drawer.ref;
  const sidebarResizeCleanupRef = useRef<(() => void) | null>(null);
  const sidebarDoubleClickRef = useRef<{ time: number; x: number } | null>(null);

  const resizeSidebar = (clientX: number) => {
    const left = asideRef.current?.getBoundingClientRect().left ?? 0;
    setSidebarWidth(constrainSidebarWidth(clientX - left, window.innerWidth));
  };
  const finishSidebarResize = () => {
    setResizingSidebar(false);
    setSidebarWidth((current) => {
      const constrained = constrainSidebarWidth(current, window.innerWidth);
      saveSidebarWidth(constrained);
      return constrained;
    });
  };
  const handleSidebarResizeStart = (event: React.MouseEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    // preventDefault on mousedown suppresses the browser's dblclick (Chromium),
    // so detect the double-click here: a repeat press at nearly the same x
    // within the double-click window resets to the default width.
    const now = Date.now();
    const previous = sidebarDoubleClickRef.current;
    sidebarDoubleClickRef.current = { time: now, x: event.clientX };
    if (previous && now - previous.time < 400 && Math.abs(event.clientX - previous.x) <= 4) {
      setSidebarWidth(DEFAULT_SIDEBAR_WIDTH);
      saveSidebarWidth(DEFAULT_SIDEBAR_WIDTH);
      return;
    }
    sidebarResizeCleanupRef.current?.();
    setResizingSidebar(true);
    // Only resize once the pointer actually moves: a plain click (or a second
    // click within the double-click window) must not shift the width.
    let lastX = event.clientX;
    const move = (moveEvent: MouseEvent) => {
      moveEvent.preventDefault();
      if (Math.abs(moveEvent.clientX - lastX) <= 1) return;
      lastX = moveEvent.clientX;
      resizeSidebar(moveEvent.clientX);
    };
    const stop = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", stop);
      sidebarResizeCleanupRef.current = null;
      finishSidebarResize();
    };
    sidebarResizeCleanupRef.current = stop;
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", stop, { once: true });
  };
  const handleSidebarResizeKey = (event: React.KeyboardEvent<HTMLDivElement>) => {
    let next = constrainSidebarWidth(sidebarWidth, window.innerWidth);
    if (event.key === "ArrowRight") next += 24;
    else if (event.key === "ArrowLeft") next -= 24;
    else if (event.key === "Home") next = MIN_SIDEBAR_WIDTH;
    else if (event.key === "End") next = constrainSidebarWidth(window.innerWidth, window.innerWidth);
    else return;
    event.preventDefault();
    const constrained = constrainSidebarWidth(next, window.innerWidth);
    setSidebarWidth(constrained);
    saveSidebarWidth(constrained);
  };

  useEffect(() => {
    const onResize = () =>
      setViewportWidth(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      sidebarResizeCleanupRef.current?.();
    };
  }, []);

  const renderedWidth = constrainSidebarWidth(sidebarWidth, viewportWidth);
  const maxSidebarWidth = constrainSidebarWidth(window.innerWidth, window.innerWidth);
  const narrow = drawer.narrow;
  const closeDrawer = drawer.setOpen;

  useEffect(() => {
    setProjectPreferences(loadProjectSidebarPreferences(undefined, accountScope));
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

  // Quick chats stay open by default, and navigating to a conversation reveals
  // its project. Everything else remains collapsed until the user asks for it,
  // which keeps large project libraries scannable without hiding active work.
  useEffect(() => {
    if (!activeGroupKey) return;
    setExpandedProjectKeys((current) => {
      if (current.has(activeGroupKey)) return current;
      return new Set([...current, activeGroupKey]);
    });
  }, [activeGroupKey]);

  const toggleProjectExpanded = useCallback((key: string) => {
    setExpandedProjectKeys((current) => {
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
      filter || expandedProjectKeys.has(group.key)
        ? group.convos.map((conversation) => conversation.id)
        : []
    ),
    [expandedProjectKeys, filter, groups],
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
  }, [closeDrawer, openConversation, setSelection]);

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

  const commitProjectPreferences = (next: ProjectSidebarPreferences) => {
    saveProjectSidebarPreferences(next, undefined, accountScope);
    setProjectPreferences(next);
  };

  const movePinnedProjectTo = useCallback((key: string, index: number, label: string) => {
    const next = withPinnedProjectMoved(projectPreferences, key, index);
    if (next === projectPreferences) return;
    saveProjectSidebarPreferences(next, undefined, accountScope);
    setProjectPreferences(next);
    announce(`Project ${label} moved to position ${index + 1} of ${next.pinned.length}.`);
  }, [accountScope, projectPreferences]);

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
    const estimatedHeight = group.path ? 212 : 152;
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

  const archiveSection = <SidebarArchive archivedConvos={archivedConvos} open={archivedOpen}
    onToggle={() => setArchivedOpen((open) => !open)} mutatingIds={mutatingIds}
    mutationKind={conversationMutation?.kind} onRestore={restoreAndOpenConversation} onDelete={deleteArchivedConversation} />;

  if (narrow ? !drawer.open : collapsed) {
    return (
      <div className="flex w-12 shrink-0 flex-col items-center gap-1 bg-bg-secondary py-2">
        <SidebarHeader rail onToggle={() => narrow ? drawer.setOpen(true) : setCollapsed(false)} filter={filter} onFilter={setFilter}
          searchRef={searchInputRef} artifactCount={artifactCount} onOpenArtifacts={onOpenArtifacts} />
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
        searchRef={searchInputRef} artifactCount={artifactCount} onOpenArtifacts={onOpenArtifacts} />

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
        className="min-h-0 flex-1 overflow-y-auto px-2 pb-14"
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
        <h2 className="px-2 pb-2 pt-1 text-xs font-semibold uppercase tracking-wider text-ink-muted">Projects & chats</h2>

        {conversations.length === 0 && groups.length === 0 ? (
          <p className="px-1 py-6 text-center text-sm text-ink-faint">
            {conversationsLoading ? "Loading conversations…" : "Choose New session to open a project, or start a quick chat."}
          </p>
        ) : visible.length === 0 && groups.length === 0 ? (
          <p className="px-1 py-6 text-center text-sm text-ink-faint">
            No projects or chats match “{filter}”.
          </p>
        ) : (
          <div className="flex flex-col">
            <AnimatePresence initial={false} mode="popLayout">
              {groups.map((g) => {
                const expanded = Boolean(filter) || expandedProjectKeys.has(g.key);
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
                              className="ml-5 overflow-hidden border-l border-border-subtle pl-1"
                            >
                              {g.convos.length === 0 && <div className="px-2 py-2 text-sm text-ink-muted">
                                <p>No sessions yet.</p>
                                <button type="button" onClick={() => startProjectSession(g)} className="mt-1 text-accent hover:underline">Start a session</button>
                              </div>}
                              <div className="flex flex-col gap-0.5 py-0.5">
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
        <AnimatePresence initial={false}>
          {(selectedIds.size > 0 || conversationMutation) && (
            <m.div
              key="conversation-selection-toolbar"
              {...accessibleMotion(RISE, reduceMotion)}
              className="absolute inset-x-0 bottom-full z-10 bg-bg-secondary px-3 py-2 shadow-soft"
            >
              {deleteConfirming && selectedIds.size > 0 ? (
                <div className="flex items-center gap-1.5 text-sm text-ink-muted">
                  <span className="min-w-0 flex-1">Delete {selectedIds.size} permanently?</span>
                  <button
                    ref={deleteConfirmRef}
                    onClick={() => {
                      setDeleteConfirming(false);
                      deleteSelectedWithFocus();
                    }}
                    className="rounded-lg bg-danger/10 px-2 py-1.5 font-medium text-danger transition hover:bg-danger/20"
                  >
                    Delete
                  </button>
                  <button
                    onClick={() => setDeleteConfirming(false)}
                    className="rounded-lg px-2 py-1.5 text-ink-muted transition hover:bg-bg-hover hover:text-ink"
                  >
                    Cancel
                  </button>
                </div>
              ) : (
                <div className="flex items-center gap-1.5 text-sm text-ink-muted">
                  <span className="min-w-0 flex-1 tabular-nums">{selectionStatus}</span>
                  {selectedIds.size > 0 && (
                    <>
                      <button
                        disabled={mutationInFlight}
                        onClick={archiveSelectedWithFocus}
                        className="flex items-center gap-1.5 rounded-lg px-2 py-1.5 transition hover:bg-accent-subtle hover:text-ink disabled:cursor-wait disabled:opacity-50"
                        title="Archive selected"
                      >
                        <Archive className="size-3.5" /> Archive
                      </button>
                      <button
                        disabled={mutationInFlight}
                        onClick={() => setDeleteConfirming(true)}
                        className="flex items-center gap-1.5 rounded-lg px-2 py-1.5 transition hover:bg-danger/10 hover:text-danger disabled:cursor-wait disabled:opacity-50"
                        title="Delete selected permanently"
                      >
                        <Trash2 className="size-3.5" /> Delete
                      </button>
                      <button
                        disabled={mutationInFlight}
                        onClick={() => setSelection(new Set())}
                        aria-label="Clear selection"
                        title="Clear selection"
                        className="grid size-7 place-items-center rounded-lg text-ink-faint transition hover:bg-bg-hover hover:text-ink disabled:cursor-wait disabled:opacity-50"
                      >
                        <X className="size-3.5" />
                      </button>
                    </>
                  )}
                </div>
              )}
            </m.div>
          )}
        </AnimatePresence>
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

      {projectMenu && (
        <ProjectActionsMenu
          key={projectMenu.group.key}
          group={projectMenu.group}
          position={projectMenu.position}
          pinned={projectPreferences.pinned.includes(projectMenu.group.key)}
          moveDestinations={projectMoveDestinations(
            pinnedGroups,
            projectMenu.group.key,
            projectPreferences.pinned,
          )}
          onClose={() => {
            projectMenu.trigger.focus();
            setProjectMenu(null);
          }}
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
          onRemove={() => {
            archiveProjectChats(projectMenu.group);
            const path = projectMenu.group.path;
            if (path) {
              if (defaultProject.trim() === path) setLocalSettings({ cwd: "" });
              const next = removeRecentProject(path, accountScope);
              useSessionStore.setState({ recentProjects: next });
            }
            commitProjectPreferences(
              withoutProjectPreferences(projectPreferences, projectMenu.group.key),
            );
          }}
        />
      )}
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
        onDoubleClick={() => {
          setSidebarWidth(DEFAULT_SIDEBAR_WIDTH);
          saveSidebarWidth(DEFAULT_SIDEBAR_WIDTH);
        }}
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
