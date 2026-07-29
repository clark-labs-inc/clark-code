import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import {
  Plus, MessageSquare, Archive, ArchiveRestore, ChevronRight, PanelLeftClose, PanelLeft,
  FolderPlus, Search, X, Trash2, Loader2, Library,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { liveSessions } from "../store/sessionStore.runtime";
import { projectName, removeRecentProject } from "../lib/localAgent";
import { useIsNarrow } from "../lib/responsive";
import { fuzzyFilter } from "../lib/fuzzy";
import { stableRankMap } from "../lib/stableOrder";
import { cn } from "../lib/cn";
import { getBridge } from "../core-bridge/bridge";
import { openProjectPath } from "../lib/openPath";
import { loadSshHosts } from "../lib/sshHosts";
import {
  groupSidebarProjects,
  loadProjectSidebarPreferences,
  saveProjectSidebarPreferences,
  withProjectAlias,
  withProjectPinned,
  withoutProjectPreferences,
  type ProjectGroup,
  type ProjectSidebarPreferences,
} from "../lib/projectSidebar";
import {
  adjacentConversationId,
  conversationMutationStatusLabel,
  conversationRangeIds,
  type SidebarConversationMutationKind,
} from "../lib/sidebarConversationInteractions";
import { DUR, EASE } from "../lib/motion";
import { ProfileMenu } from "./ProfileMenu";
import {
  ProjectActionsMenu,
  ProjectHeader,
  type ProjectMenuPosition,
} from "./ProjectActionsMenu";
import type { ConversationMeta } from "../lib/history";

type ConversationSelectionIntent = "open" | "toggle" | "range";

interface SidebarScrollAnchor {
  id: string;
  offset: number;
  order: string[];
  index: number;
}

function ConversationRow({
  c,
  active,
  streaming,
  opening,
  selected,
  mutation,
  onSelect,
  onRangeStep,
  onArchive,
  onContextMenu,
}: {
  c: ConversationMeta;
  active: boolean;
  /** A run is currently streaming in this conversation. */
  streaming: boolean;
  /** This conversation is currently being (re)opened. */
  opening: boolean;
  /** In the sidebar's Shift-click selection. */
  selected: boolean;
  mutation: SidebarConversationMutationKind | null;
  onSelect: (id: string, intent: ConversationSelectionIntent, additive?: boolean) => void;
  onRangeStep: (id: string, direction: -1 | 1) => void;
  onArchive: (id: string) => void;
  onContextMenu: (e: React.MouseEvent, id: string) => void;
}) {
  const rename = useSessionStore((s) => s.renameConversation);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(c.title);
  const mutating = mutation !== null;

  const commit = () => {
    setEditing(false);
    rename(c.id, draft);
  };

  return (
    <div
      onContextMenu={(e) => {
        if (!mutating) onContextMenu(e, c.id);
      }}
      aria-busy={mutating || undefined}
      className={cn(
        "group relative flex min-h-7 items-center gap-1 rounded-lg px-2 py-0.5 text-sm transition duration-150 ease-clark",
        mutating && "opacity-60",
        selected
          ? "bg-bg-tertiary text-ink ring-1 ring-border"
          : active || opening
            ? "bg-bg-tertiary text-ink"
            : "text-ink-secondary hover:bg-bg-hover hover:text-ink",
      )}
    >
      {editing ? (
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <MessageSquare className="size-3.5 shrink-0 text-ink-faint" />
          <input
            autoFocus
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => {
              if (e.key === "Enter") commit();
              if (e.key === "Escape") {
                setDraft(c.title);
                setEditing(false);
              }
            }}
            aria-label="Rename conversation"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            className="composer-input min-w-0 flex-1 rounded-md bg-bg-sunken px-1.5 py-0.5 text-sm text-ink outline-none ring-1 ring-border-subtle"
          />
        </div>
      ) : (
        <button
          onClick={(e) => {
            const additive = e.metaKey || e.ctrlKey;
            if (e.shiftKey) {
              onSelect(c.id, "range", additive);
            } else if (additive) {
              onSelect(c.id, "toggle");
            } else {
              onSelect(c.id, "open");
            }
          }}
          onKeyDown={(e) => {
            if (e.shiftKey && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
              e.preventDefault();
              onRangeStep(c.id, e.key === "ArrowDown" ? 1 : -1);
              return;
            }
            if (e.key === " " && (e.shiftKey || e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              onSelect(c.id, e.shiftKey ? "range" : "toggle", e.metaKey || e.ctrlKey);
            }
          }}
          onDoubleClick={() => {
            setDraft(c.title);
            setEditing(true);
          }}
          disabled={mutating}
          data-sidebar-conversation-button={c.id}
          aria-pressed={selected}
          aria-describedby="sidebar-selection-help"
          aria-label={
            mutating
              ? `${mutation === "archive" ? "Archiving" : mutation === "delete" ? "Deleting" : "Restoring"} ${c.title}`
              : `Conversation: ${c.title}${selected ? ", selected" : ""}`
          }
          className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
          title={`${c.title} — Shift-click or Shift+Arrow selects a range`}
        >
          {mutating || opening ? (
            <Loader2 className="size-3.5 shrink-0 animate-[spin_1s_linear_infinite] text-ink-muted" />
          ) : streaming ? (
            <span className="relative grid size-3.5 shrink-0 place-items-center" aria-hidden="true">
              <span className="absolute size-2 animate-ping rounded-full bg-accent/40" />
              <span className="size-1.5 rounded-full bg-accent" />
            </span>
          ) : selected ? (
            <span className="grid size-3 shrink-0 place-items-center" aria-hidden="true">
              <span className="size-2 rounded-sm bg-accent" />
            </span>
          ) : (
            <span className="size-3 shrink-0" aria-hidden="true" />
          )}
          <span className="min-w-0 flex-1 truncate leading-5">{c.title}</span>
        </button>
      )}
      {!editing && (
        <button
          onClick={() => onArchive(c.id)}
          disabled={mutating}
          title="Archive conversation"
          aria-label="Archive conversation"
          className="shrink-0 rounded-md p-1 text-ink-faint opacity-0 transition hover:bg-bg-sunken hover:text-ink group-hover:opacity-100 group-focus-within:opacity-100 disabled:cursor-wait"
        >
          <Archive className="size-3.5" />
        </button>
      )}
    </div>
  );
}

/** A dimmed, minimal row inside the collapsed "Archived" section. Clicking the
 *  row restores the conversation (returns it to the active list); the trash
 *  button permanently deletes it (local cache + cloud) behind an inline confirm,
 *  since that can't be undone. */
function ArchivedRow({
  c,
  mutation,
  onRestore,
  onDelete,
}: {
  c: ConversationMeta;
  mutation: SidebarConversationMutationKind | null;
  onRestore: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const mutating = mutation !== null;
  return (
    <div
      aria-busy={mutating || undefined}
      className="group flex min-h-7 w-full items-center gap-1 rounded-lg px-2 py-0.5 text-sm text-ink-faint transition hover:bg-bg-hover"
    >
      <button
        onClick={() => onRestore(c.id)}
        disabled={mutating}
        title={`Restore “${c.title}”`}
        aria-label={mutating ? `Restoring ${c.title}` : `Restore ${c.title} and open it`}
        className="flex min-w-0 flex-1 items-center gap-1.5 text-left transition hover:text-ink-secondary"
      >
        {mutating ? (
          <Loader2 className="size-3.5 shrink-0 animate-[spin_1s_linear_infinite] text-ink-muted" />
        ) : (
          <MessageSquare className="size-3.5 shrink-0 text-ink-faint" />
        )}
        <span className="min-w-0 flex-1 truncate leading-5">{c.title}</span>
        <ArchiveRestore className="size-3.5 shrink-0 opacity-0 transition group-hover:opacity-100" />
      </button>
      {confirming ? (
        <span className="flex shrink-0 items-center gap-1">
          <button
            onClick={() => onDelete(c.id)}
            disabled={mutating}
            aria-label={`Permanently delete ${c.title}`}
            className="rounded-md px-1.5 py-0.5 text-xs font-medium text-danger transition hover:bg-danger/10"
          >
            Delete
          </button>
          <button
            onClick={() => setConfirming(false)}
            aria-label="Cancel delete"
            className="rounded-md px-1.5 py-0.5 text-xs text-ink-muted transition hover:bg-bg-hover hover:text-ink"
          >
            Cancel
          </button>
        </span>
      ) : (
          <button
            onClick={() => setConfirming(true)}
            disabled={mutating}
          title="Delete permanently"
          aria-label={`Delete ${c.title} permanently`}
          className="grid size-6 shrink-0 place-items-center rounded-md text-ink-faint opacity-0 transition hover:bg-danger/10 hover:text-danger group-hover:opacity-100"
        >
          <Trash2 className="size-3.5" />
        </button>
      )}
    </div>
  );
}

/** Right-click menu for one-or-many conversations. Acts on the whole sidebar
 *  selection when the right-clicked row is part of it, otherwise just the
 *  right-clicked row. "Archive" soft-deletes; "Delete" hard-deletes (with an
 *  inline confirm — it can't be undone). */
function ConversationContextMenu({
  menu,
  count,
  onClose,
  onArchive,
  onDelete,
}: {
  menu: { x: number; y: number };
  count: number;
  onClose: () => void;
  onArchive: () => void;
  onDelete: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  const label = count > 1 ? `${count} conversations` : "conversation";
  return (
    <div
      ref={ref}
      role="menu"
      style={{ left: menu.x, top: menu.y }}
      className="popover-surface fixed z-50 w-52 rounded-xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle"
    >
      {confirming ? (
        <div className="px-1.5 py-1">
          <div className="mb-2 px-1 text-xs text-ink-muted">
            Permanently delete {count > 1 ? `these ${count} conversations` : "this conversation"}? This can't be undone.
          </div>
          <div className="flex items-center gap-1.5">
            <button
              role="menuitem"
              onClick={() => {
                onDelete();
                onClose();
              }}
              className="flex-1 rounded-lg bg-danger/10 px-2 py-1.5 text-xs font-medium text-danger transition hover:bg-danger/20"
            >
              Delete
            </button>
            <button
              role="menuitem"
              onClick={() => setConfirming(false)}
              className="flex-1 rounded-lg px-2 py-1.5 text-xs text-ink-muted transition hover:bg-bg-hover hover:text-ink"
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <>
          <button
            role="menuitem"
            onClick={() => {
              onArchive();
              onClose();
            }}
            className="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-sm text-ink-secondary transition hover:bg-accent-subtle hover:text-ink"
          >
            <Archive className="size-4" />
            Archive {label}
          </button>
          <button
            role="menuitem"
            onClick={() => setConfirming(true)}
            className="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-sm text-ink-secondary transition hover:bg-danger/10 hover:text-danger"
          >
            <Trash2 className="size-4" />
            Delete {label}
          </button>
        </>
      )}
    </div>
  );
}

export function Sidebar({
  artifactCount = 0,
  onOpenArtifacts,
}: {
  artifactCount?: number;
  onOpenArtifacts?: () => void;
}) {
  const collapsed = useSessionStore((s) => s.sidebarCollapsed);
  const setCollapsed = useSessionStore((s) => s.setSidebarCollapsed);
  const conversations = useSessionStore((s) => s.conversations);
  const conversationsLoading = useSessionStore((s) => s.conversationsLoading);
  const session = useSessionStore((s) => s.session);
  const activeProjectRoot = useSessionStore((s) => s.activeProjectRoot);
  const openingId = useSessionStore((s) => s.opening?.id ?? null);
  // Any number of conversations can be streaming at once — each busy one gets
  // its own pulsing "Working…" dot, whether or not it's on screen.
  const runningIds = useSessionStore((s) => s.runningIds);
  const newConversation = useSessionStore((s) => s.endSession);
  const selectProvider = useSessionStore((s) => s.selectProvider);
  const setProjectMode = useSessionStore((s) => s.setProjectMode);
  const setSelectedHostId = useSessionStore((s) => s.setSelectedHostId);
  const setProjectFolder = useSessionStore((s) => s.setProjectFolder);
  const setSshOpen = useSessionStore((s) => s.setSshOpen);
  const openProjectTerminal = useSessionStore((s) => s.openProjectTerminal);
  const defaultProject = useSessionStore((s) => s.localSettings.cwd);
  const setLocalSettings = useSessionStore((s) => s.setLocalSettings);
  const recentProjects = useSessionStore((s) => s.recentProjects);
  const flashNotice = useSessionStore((s) => s.flashNotice);
  const openConversation = useSessionStore((s) => s.openConversation);
  const archiveConversation = useSessionStore((s) => s.archiveConversation);
  const restoreConversation = useSessionStore((s) => s.restoreConversation);
  const deleteConversation = useSessionStore((s) => s.deleteConversation);
  const selectedIds = useSessionStore((s) => s.selectedConversationIds);
  const mutatingIds = useSessionStore((s) => s.mutatingConversationIds);
  const conversationMutation = useSessionStore((s) => s.conversationMutation);
  const setSelection = useSessionStore((s) => s.setConversationSelection);
  const archiveSelected = useSessionStore((s) => s.archiveSelectedConversations);
  const deleteSelected = useSessionStore((s) => s.deleteSelectedConversations);
  const [filter, setFilter] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const [archivedOpen, setArchivedOpen] = useState(false);
  const [deleteConfirming, setDeleteConfirming] = useState(false);
  const [projectPreferences, setProjectPreferences] = useState<ProjectSidebarPreferences>(
    () => loadProjectSidebarPreferences(),
  );
  const [projectMenu, setProjectMenu] = useState<{
    group: ProjectGroup;
    position: ProjectMenuPosition;
  } | null>(null);
  // The right-click menu: positioned at the cursor; acts on the selection when
  // the right-clicked row is in it, else just that row.
  const [menu, setMenu] = useState<{ x: number; y: number; ids: string[] } | null>(null);
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
  // Below this width the full sidebar would crowd out the conversation, so it
  // auto-collapses to the icon rail (and can't be expanded until there's room).
  const narrow = useIsNarrow(768);

  // No artificial cap here: a hard limit combined with ranking by recency is
  // what made the WHOLE list reshuffle once you passed it (any update landed in
  // the top-N and triggered a full re-sort). Keep every conversation; order is
  // stabilized below, and search narrows it when you actually filter.
  const visible = useMemo(
    () =>
      fuzzyFilter(
        conversations,
        filter,
        (c) => `${c.title} ${c.project ? projectName(c.project) : ""} ${c.remoteHost ?? ""}`,
        5000,
      ).map((m) => m.item),
    [conversations, filter],
  );
  // One stable rank per conversation, shared by the group + row ordering so a
  // parallel run's timestamp bump never reshuffles the list mid-session.
  const rank = useMemo(() => {
    const m = stableRankMap(visible);
    return (id: string) => m.get(id) ?? 0;
  }, [visible]);
  // Archived conversations are hidden from the project groups and collected into
  // their own collapsed section (search still matches across both).
  const activeConvos = useMemo(() => visible.filter((c) => !c.archived), [visible]);
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
    () => groupSidebarProjects(activeConvos, rememberedProjects, rank, projectPreferences, filter),
    [activeConvos, rememberedProjects, rank, projectPreferences, filter],
  );
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
    () => groups.flatMap((group) => group.convos.map((conversation) => conversation.id)),
    [groups],
  );
  const activeConversationSignature = activeConversationIds.join("\u0000");

  // Reading every row's geometry during a scroll forces synchronous layout on
  // large histories. We only need an anchor immediately before an operation
  // changes the list, so capture it at that boundary instead.
  const captureScrollAnchor = () => {
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
  };

  const focusConversation = (id: string): boolean => {
    const container = conversationListRef.current;
    if (!container) return false;
    const button = Array.from(
      container.querySelectorAll<HTMLButtonElement>("[data-sidebar-conversation-button]"),
    ).find((candidate) => candidate.dataset.sidebarConversationButton === id);
    if (!button) return false;
    button.focus({ preventScroll: true });
    button.scrollIntoView({ block: "nearest" });
    return true;
  };

  const requestMutationFocus = (id: string | null) => {
    mutationFocusIdRef.current = id;
    mutationFocusOrderRef.current = activeConversationIds;
  };

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

  const selectConversation = (
    id: string,
    intent: ConversationSelectionIntent,
    additive = false,
  ) => {
    if (mutatingIds.has(id)) return;
    if (intent === "open") {
      selectionAnchorRef.current = id;
      setSelection(new Set());
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
  };

  const extendSelectionWithKeyboard = (id: string, direction: -1 | 1) => {
    if (!selectionAnchorRef.current || !activeConversationIds.includes(selectionAnchorRef.current)) {
      selectionAnchorRef.current = id;
    }
    const target = adjacentConversationId(activeConversationIds, id, direction);
    if (!target) return;
    selectConversation(target, "range");
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(() => focusConversation(target));
    else focusConversation(target);
  };

  const restoreAndOpenConversation = (id: string) => {
    captureScrollAnchor();
    restoredFocusIdRef.current = id;
    void restoreConversation(id);
  };
  const archiveConversationWithFocus = (id: string) => {
    captureScrollAnchor();
    requestMutationFocus(id);
    void archiveConversation(id);
  };
  const deleteArchivedConversation = (id: string) => {
    captureScrollAnchor();
    requestMutationFocus(null);
    void deleteConversation(id);
  };
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
    saveProjectSidebarPreferences(next);
    setProjectPreferences(next);
  };

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
    setProjectMenu({ group, position: { left, top } });
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
      const destination = group.remoteHost;
      const candidates = loadSshHosts().filter(
        (candidate) => candidate.host.trim() === destination,
      );
      const host =
        candidates.find((candidate) => candidate.remoteRoot.trim() === group.remoteRoot) ??
        candidates[0];
      if (!host) {
        flashNotice(`Reconnect ${group.label} in Remote hosts to start a new session.`);
        setSshOpen(true);
        return;
      }
      selectProvider("local");
      setProjectMode("remote");
      setSelectedHostId(host.id);
      newConversation();
      return;
    }
    if (group.path) {
      selectProvider("local");
      setProjectMode("local");
      setProjectFolder(group.path);
      newConversation();
    }
  };

  const openContextMenu = (e: React.MouseEvent, id: string) => {
    e.preventDefault();
    if (mutatingIds.size > 0) return;
    setProjectMenu(null);
    // Act on the whole selection when the right-clicked row is part of it;
    // otherwise the action targets just this row (and the selection becomes it,
    // so the visual matches what the menu will act on).
    if (selectedIds.has(id)) {
      selectionAnchorRef.current = id;
      setMenu({ x: e.clientX, y: e.clientY, ids: [...selectedIds] });
    } else {
      selectionAnchorRef.current = id;
      setSelection(new Set([id]));
      setMenu({ x: e.clientX, y: e.clientY, ids: [id] });
    }
  };

  if (collapsed || narrow) {
    return (
      <div className="flex w-12 shrink-0 flex-col items-center gap-1 border-r border-border-subtle bg-bg-secondary py-2">
        {!narrow && (
          <button
            onClick={() => setCollapsed(false)}
            aria-label="Expand sidebar"
            className="grid size-8 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover hover:text-ink"
          >
            <PanelLeft className="size-4" />
          </button>
        )}
        <button
          onClick={() => newConversation()}
          aria-label="New session"
          title="New session"
          className="grid size-8 place-items-center rounded-lg text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
        >
          <Plus className="size-4" />
        </button>
        <button
          onClick={() => void openProjectTerminal()}
          aria-label="New project"
          title="New project — choose a folder and open a terminal in it"
          className="grid size-8 place-items-center rounded-lg text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
        >
          <FolderPlus className="size-4" />
        </button>
        {onOpenArtifacts && (
          <button
            onClick={onOpenArtifacts}
            aria-label={`Artifacts, ${artifactCount}`}
            title={`Artifacts (${artifactCount})`}
            className="grid size-8 place-items-center rounded-lg text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
          >
            <Library className="size-4" />
          </button>
        )}
        <div className="mt-auto">
          <ProfileMenu variant="rail" />
        </div>
      </div>
    );
  }

  return (
    <aside className="flex w-[17rem] shrink-0 flex-col border-r border-border-subtle bg-bg-secondary text-sm leading-5">
      <div className="flex min-h-12 shrink-0 items-center gap-1 px-3 py-1">
        <span className="truncate text-base font-semibold tracking-[-0.01em] text-ink">Clark Code</span>
        <button
          onClick={() => setSearchOpen((open) => !open)}
          aria-label="Search conversations"
          title="Search conversations"
          className="ml-auto grid size-8 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover hover:text-ink"
        >
          <Search className="size-4" />
        </button>
        <button
          onClick={() => setCollapsed(true)}
          aria-label="Collapse sidebar"
          className="grid size-8 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover hover:text-ink"
        >
          <PanelLeftClose className="size-4" />
        </button>
      </div>

      <div className="px-2 pb-2">
        <button
          onClick={() => newConversation()}
          className="flex min-h-8 w-full items-center gap-2.5 rounded-lg px-2 py-1 text-sm font-medium text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
        >
          <Plus className="size-4" /> New session
        </button>
        <button
          onClick={() => void openProjectTerminal()}
          title="Choose a folder, set it as the current project, and open a terminal in it"
          className="flex min-h-8 w-full items-center gap-2.5 rounded-lg px-2 py-1 text-sm font-medium text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
        >
          <FolderPlus className="size-4" /> New project…
        </button>
        {/* Always rendered (even at 0) so the row's height never pops in/out
            when switching conversations — a conditional row made the whole
            list below jump up/down as artifact counts differed per chat. The
            badge keeps a fixed slot; at 0 it's dimmed to match the icon. */}
        {onOpenArtifacts && (
          <button
            type="button"
            onClick={onOpenArtifacts}
            className="mt-1 flex min-h-8 w-full items-center gap-2.5 rounded-lg px-2 py-1 text-sm font-medium text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
          >
            <Library className="size-4" />
            <span>Artifacts</span>
            <span
              className={cn(
                "ml-auto min-w-5 rounded-full px-1.5 text-center text-xs tabular-nums",
                artifactCount > 0 ? "bg-chip text-ink-faint" : "text-ink-faint",
              )}
            >
              {artifactCount}
            </span>
          </button>
        )}
      </div>

      {(searchOpen || filter) && (
        <div className="px-2 pb-2">
          <div className="flex min-h-8 items-center gap-2 rounded-lg bg-bg-primary px-2.5 py-1 ring-1 ring-border-subtle transition focus-within:ring-border-strong">
            <Search className="size-3.5 shrink-0 text-ink-faint" />
            <input
              autoFocus
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Search conversations…"
              aria-label="Search conversations"
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  setFilter("");
                  setSearchOpen(false);
                }
              }}
              className="composer-input min-w-0 flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-ink-faint"
            />
            {filter && (
              <button
                onClick={() => setFilter("")}
                aria-label="Clear search"
                className="grid size-6 shrink-0 place-items-center rounded-md text-ink-faint transition hover:bg-bg-hover hover:text-ink"
              >
                <X className="size-3" />
              </button>
            )}
          </div>
        </div>
      )}

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
        {conversations.length === 0 && groups.length === 0 ? (
          <p className="px-1 py-6 text-center text-xs text-ink-faint">
            {conversationsLoading ? "Loading conversations…" : "Your conversations will show up here."}
          </p>
        ) : visible.length === 0 && groups.length === 0 ? (
          <p className="px-1 py-6 text-center text-xs text-ink-faint">
            No conversations match “{filter}”.
          </p>
        ) : (
          <div className="flex flex-col">
            {groups.length > 0 && (
              <div className="px-2 pb-1 pt-2 text-xs font-medium text-ink-faint">Projects</div>
            )}
            <AnimatePresence initial={false} mode="popLayout">
              {groups.map((g) => (
                <motion.section
                  key={g.key}
                  layout={reduceMotion ? false : "position"}
                  initial={reduceMotion ? false : { opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={reduceMotion ? undefined : { opacity: 0, y: -2 }}
                  transition={{ duration: reduceMotion ? 0 : DUR.fast, ease: EASE.out }}
                >
                  <ProjectHeader
                    group={g}
                    menuOpen={projectMenu?.group.key === g.key}
                    onOpenMenu={(button) => openProjectMenu(g, button)}
                    onNewSession={() => startProjectSession(g)}
                  />
                  <div className="flex flex-col">
                    <AnimatePresence initial={false} mode="popLayout">
                      {g.convos.map((c) => {
                        const mutation = mutatingIds.has(c.id)
                          ? conversationMutation?.kind ?? "archive"
                          : null;
                        return (
                          <motion.div
                            key={c.id}
                            data-sidebar-conversation-id={c.id}
                            layout={reduceMotion ? false : "position"}
                            initial={reduceMotion ? false : { opacity: 0, y: 4 }}
                            animate={{ opacity: 1, y: 0 }}
                            exit={reduceMotion ? undefined : { opacity: 0, y: -2 }}
                            transition={{ duration: reduceMotion ? 0 : DUR.fast, ease: EASE.out }}
                          >
                            <ConversationRow
                              c={c}
                              active={session?.id === c.id}
                              streaming={runningIds.includes(c.id)}
                              opening={openingId === c.id}
                              selected={selectedIds.has(c.id)}
                              mutation={mutation}
                              onSelect={selectConversation}
                              onRangeStep={extendSelectionWithKeyboard}
                              onArchive={archiveConversationWithFocus}
                              onContextMenu={openContextMenu}
                            />
                          </motion.div>
                        );
                      })}
                    </AnimatePresence>
                  </div>
                </motion.section>
              ))}
            </AnimatePresence>

            {groups.length === 0 && archivedConvos.length > 0 && (
              <p className="px-1 py-6 text-center text-xs text-ink-faint">
                No active conversations.
              </p>
            )}
          </div>
        )}
      </div>

      {/* This header is always present so archiving and restoring never insert
          or remove a sidebar control. The tray itself only grows after an
          explicit click and its height is animated. */}
      <div className="shrink-0 border-t border-border-subtle">
        <button
          onClick={() => setArchivedOpen((open) => !open)}
          disabled={archivedConvos.length === 0}
          aria-controls="archived-conversations"
          aria-expanded={archivedConvos.length > 0 ? archivedOpen : undefined}
          className="flex min-h-9 w-full items-center gap-2 px-4 py-1 text-sm font-medium text-ink-muted transition hover:text-ink disabled:cursor-default disabled:opacity-55"
        >
          <ChevronRight
            className={`size-3 shrink-0 transition-transform ${archivedOpen && archivedConvos.length > 0 ? "rotate-90" : ""}`}
          />
          <span>Archived</span>
          <span className="ml-auto shrink-0 text-xs font-normal tabular-nums text-ink-faint">
            {archivedConvos.length}
          </span>
        </button>
        <AnimatePresence initial={false}>
          {archivedOpen && archivedConvos.length > 0 && (
            <motion.div
              id="archived-conversations"
              initial={reduceMotion ? false : { height: 0, opacity: 0 }}
              animate={{ height: "auto", opacity: 1 }}
              exit={reduceMotion ? undefined : { height: 0, opacity: 0 }}
              transition={{ duration: reduceMotion ? 0 : DUR.base, ease: EASE.out }}
              className="overflow-hidden"
            >
              <div className="flex max-h-56 flex-col gap-0.5 overflow-y-auto px-2 pb-2">
                <AnimatePresence initial={false} mode="popLayout">
                  {archivedConvos.map((c) => {
                    const mutation = mutatingIds.has(c.id)
                      ? conversationMutation?.kind ?? "restore"
                      : null;
                    return (
                      <motion.div
                        key={c.id}
                        layout={reduceMotion ? false : "position"}
                        initial={reduceMotion ? false : { opacity: 0, y: 4 }}
                        animate={{ opacity: 1, y: 0 }}
                        exit={reduceMotion ? undefined : { opacity: 0, y: -2 }}
                        transition={{ duration: reduceMotion ? 0 : DUR.fast, ease: EASE.out }}
                      >
                        <ArchivedRow
                          c={c}
                          mutation={mutation}
                          onRestore={restoreAndOpenConversation}
                          onDelete={deleteArchivedConversation}
                        />
                      </motion.div>
                    );
                  })}
                </AnimatePresence>
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      <div className="relative shrink-0">
        <AnimatePresence initial={false}>
          {(selectedIds.size > 0 || conversationMutation) && (
            <motion.div
              key="conversation-selection-toolbar"
              initial={reduceMotion ? false : { opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={reduceMotion ? undefined : { opacity: 0, y: 8 }}
              transition={{ duration: reduceMotion ? 0 : DUR.fast, ease: EASE.out }}
              className="absolute inset-x-0 bottom-full z-10 border-t border-border-subtle bg-bg-secondary px-3 py-2 shadow-[0_-8px_16px_rgb(0_0_0_/_0.04)]"
            >
              {deleteConfirming && selectedIds.size > 0 ? (
                <div className="flex items-center gap-1.5 text-xs text-ink-muted">
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
                <div className="flex items-center gap-1.5 text-xs text-ink-muted">
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
            </motion.div>
          )}
        </AnimatePresence>
        <div className="border-t border-border-subtle px-2 py-1">
          <ProfileMenu variant="sidebar" />
        </div>
      </div>

      {menu && (
        <ConversationContextMenu
          menu={{ x: menu.x, y: menu.y }}
          count={menu.ids.length}
          onClose={() => setMenu(null)}
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
          onClose={() => setProjectMenu(null)}
          onPin={(pinned) =>
            commitProjectPreferences(
              withProjectPinned(projectPreferences, projectMenu.group.key, pinned),
            )
          }
          onReveal={() => {
            const path = projectMenu.group.path;
            if (path) void openProjectPath(path, "", true);
          }}
          onCreateWorktree={async (name) => {
            const path = projectMenu.group.path;
            if (!path) throw new Error("A local project folder is required.");
            const bridge = await getBridge();
            if (!bridge.createPermanentWorktree) {
              throw new Error("Permanent worktrees are available in the desktop app.");
            }
            const createdPath = await bridge.createPermanentWorktree(path, name);
            await openProjectTerminal(createdPath);
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
              const next = removeRecentProject(path);
              useSessionStore.setState({ recentProjects: next });
            }
            commitProjectPreferences(
              withoutProjectPreferences(projectPreferences, projectMenu.group.key),
            );
          }}
        />
      )}
    </aside>
  );
}
