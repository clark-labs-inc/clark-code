import { useEffect, useMemo, useRef, useState } from "react";
import {
  Plus, MessageSquare, Archive, ArchiveRestore, ChevronRight, PanelLeftClose, PanelLeft,
  FolderPlus, Search, X, Trash2, Loader2, Library,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { projectName, removeRecentProject } from "../lib/localAgent";
import { useIsNarrow } from "../lib/responsive";
import { fuzzyFilter } from "../lib/fuzzy";
import { stableRankMap } from "../lib/stableOrder";
import { cn } from "../lib/cn";
import { getBridge } from "../core-bridge/bridge";
import { openProjectPath } from "../lib/openPath";
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
import { ProfileMenu } from "./ProfileMenu";
import {
  ProjectActionsMenu,
  ProjectHeader,
  type ProjectMenuPosition,
} from "./ProjectActionsMenu";
import type { ConversationMeta } from "../lib/history";

function ConversationRow({
  c,
  active,
  streaming,
  opening,
  selected,
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
  onContextMenu: (e: React.MouseEvent, id: string) => void;
}) {
  const open = useSessionStore((s) => s.openConversation);
  const archive = useSessionStore((s) => s.archiveConversation);
  const rename = useSessionStore((s) => s.renameConversation);
  const toggleSelection = useSessionStore((s) => s.toggleConversationSelection);
  const setSelection = useSessionStore((s) => s.setConversationSelection);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(c.title);

  const commit = () => {
    setEditing(false);
    rename(c.id, draft);
  };

  return (
    <div
      onContextMenu={(e) => onContextMenu(e, c.id)}
      className={cn(
        "group relative flex h-7 items-center gap-1 rounded-lg px-2 text-sm transition duration-150 ease-clark",
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
            // Shift-click toggles multi-select instead of opening; a plain
            // click clears any selection and opens (the normal action).
            if (e.shiftKey) {
              toggleSelection(c.id);
            } else {
              setSelection(new Set());
              void open(c.id);
            }
          }}
          onDoubleClick={() => {
            setDraft(c.title);
            setEditing(true);
          }}
          className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
          title={`${c.title} — double-click to rename · shift-click to select`}
        >
          {opening ? (
            <Loader2 className="size-3.5 shrink-0 animate-[spin_1s_linear_infinite] text-ink-muted" />
          ) : streaming ? (
            <span className="relative grid size-3.5 shrink-0 place-items-center" aria-label="Running">
              <span className="absolute size-2 animate-ping rounded-full bg-accent/40" />
              <span className="size-1.5 rounded-full bg-accent" />
            </span>
          ) : selected ? (
            <span className="grid size-3 shrink-0 place-items-center" aria-label="Selected">
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
          onClick={() => archive(c.id)}
          title="Archive conversation"
          aria-label="Archive conversation"
          className="shrink-0 rounded-md p-1 text-ink-faint opacity-0 transition hover:bg-bg-sunken hover:text-ink group-hover:opacity-100 group-focus-within:opacity-100"
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
function ArchivedRow({ c }: { c: ConversationMeta }) {
  const restore = useSessionStore((s) => s.restoreConversation);
  const del = useSessionStore((s) => s.deleteConversation);
  const [confirming, setConfirming] = useState(false);
  return (
    <div className="group flex h-7 w-full items-center gap-1 rounded-lg px-2 text-sm text-ink-faint transition hover:bg-bg-hover">
      <button
        onClick={() => restore(c.id)}
        title={`Restore “${c.title}”`}
        aria-label={`Restore ${c.title}`}
        className="flex min-w-0 flex-1 items-center gap-1.5 text-left transition hover:text-ink-secondary"
      >
        <MessageSquare className="size-3.5 shrink-0 text-ink-faint" />
        <span className="min-w-0 flex-1 truncate leading-5">{c.title}</span>
        <ArchiveRestore className="size-3.5 shrink-0 opacity-0 transition group-hover:opacity-100" />
      </button>
      {confirming ? (
        <span className="flex shrink-0 items-center gap-1">
          <button
            onClick={() => del(c.id)}
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
  const openingId = useSessionStore((s) => s.opening?.id ?? null);
  // Any number of conversations can be streaming at once — each busy one gets
  // its own pulsing "Working…" dot, whether or not it's on screen.
  const runningIds = useSessionStore((s) => s.runningIds);
  const newConversation = useSessionStore((s) => s.endSession);
  const openProjectTerminal = useSessionStore((s) => s.openProjectTerminal);
  const defaultProject = useSessionStore((s) => s.localSettings.cwd);
  const setLocalSettings = useSessionStore((s) => s.setLocalSettings);
  const recentProjects = useSessionStore((s) => s.recentProjects);
  const flashNotice = useSessionStore((s) => s.flashNotice);
  const selectedIds = useSessionStore((s) => s.selectedConversationIds);
  const setSelection = useSessionStore((s) => s.setConversationSelection);
  const archiveSelected = useSessionStore((s) => s.archiveSelectedConversations);
  const deleteSelected = useSessionStore((s) => s.deleteSelectedConversations);
  const [filter, setFilter] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const [archivedOpen, setArchivedOpen] = useState(false);
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
    setSelection(new Set(group.convos.map((conversation) => conversation.id)));
    archiveSelected();
  };

  const openContextMenu = (e: React.MouseEvent, id: string) => {
    e.preventDefault();
    setProjectMenu(null);
    // Act on the whole selection when the right-clicked row is part of it;
    // otherwise the action targets just this row (and the selection becomes it,
    // so the visual matches what the menu will act on).
    if (selectedIds.has(id)) {
      setMenu({ x: e.clientX, y: e.clientY, ids: [...selectedIds] });
    } else {
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
          <ProfileMenu />
        </div>
      </div>
    );
  }

  return (
    <aside className="flex w-[17rem] shrink-0 flex-col border-r border-border-subtle bg-bg-secondary text-[13px] leading-5">
      <div className="flex h-12 shrink-0 items-center gap-1 px-3">
        <span className="truncate text-[15px] font-semibold tracking-[-0.01em] text-ink">Clark Code</span>
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
          className="flex h-8 w-full items-center gap-2.5 rounded-lg px-2 text-sm font-medium text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
        >
          <Plus className="size-4" /> New session
        </button>
        <button
          onClick={() => void openProjectTerminal()}
          title="Choose a folder, set it as the current project, and open a terminal in it"
          className="flex h-8 w-full items-center gap-2.5 rounded-lg px-2 text-sm font-medium text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
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
            className="mt-1 flex h-8 w-full items-center gap-2.5 rounded-lg px-2 text-sm font-medium text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
          >
            <Library className="size-4" />
            <span>Artifacts</span>
            <span
              className={cn(
                "ml-auto min-w-5 rounded-full px-1.5 text-center text-xs tabular-nums",
                artifactCount > 0 ? "bg-chip text-ink-faint" : "text-ink-faint/50",
              )}
            >
              {artifactCount}
            </span>
          </button>
        )}
      </div>

      {(searchOpen || filter) && (
        <div className="px-2 pb-2">
          <div className="flex h-8 items-center gap-2 rounded-lg bg-bg-primary px-2.5 ring-1 ring-border-subtle transition focus-within:ring-border-strong">
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
              className="composer-input min-w-0 flex-1 bg-transparent text-[13px] text-ink outline-none placeholder:text-ink-faint"
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

      <div
        className="min-h-0 flex-1 overflow-y-auto px-2 pb-4"
        onClick={(e) => {
          // A plain click on empty list space (not on a row) clears the
          // Shift-click selection.
          if (e.target === e.currentTarget && selectedIds.size > 0) setSelection(new Set());
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
            {groups.map((g) => (
              <section key={g.key}>
                <ProjectHeader
                  group={g}
                  menuOpen={projectMenu?.group.key === g.key}
                  onOpenMenu={(button) => openProjectMenu(g, button)}
                  onOpenTerminal={(path) => void openProjectTerminal(path)}
                />
                <div className="flex flex-col">
                  {g.convos.map((c) => (
                    <ConversationRow
                      key={c.id}
                      c={c}
                      active={session?.id === c.id}
                      streaming={runningIds.includes(c.id)}
                      opening={openingId === c.id}
                      selected={selectedIds.has(c.id)}
                      onContextMenu={openContextMenu}
                    />
                  ))}
                </div>
              </section>
            ))}

            {groups.length === 0 && archivedConvos.length > 0 && (
              <p className="px-1 py-6 text-center text-xs text-ink-faint">
                No active conversations.
              </p>
            )}
          </div>
        )}
      </div>

      {/* Archived lives outside the scrollable list so it never gets pushed
          below the fold by a long project list: the toggle stays pinned above
          the profile row, and expanding it opens a bounded, scrollable tray. */}
      {archivedConvos.length > 0 && (
        <div className="shrink-0 border-t border-border-subtle">
          <button
            onClick={() => setArchivedOpen((o) => !o)}
            aria-expanded={archivedOpen}
            className="flex h-9 w-full items-center gap-2 px-4 text-sm font-medium text-ink-muted transition hover:text-ink"
          >
            <ChevronRight
              className={`size-3 shrink-0 transition-transform ${archivedOpen ? "rotate-90" : ""}`}
            />
            <span>Archived</span>
            <span className="ml-auto shrink-0 text-xs font-normal tabular-nums text-ink-faint">
              {archivedConvos.length}
            </span>
          </button>
          {archivedOpen && (
            <div className="flex max-h-56 flex-col gap-0.5 overflow-y-auto px-2 pb-2">
              {archivedConvos.map((c) => (
                <ArchivedRow key={c.id} c={c} />
              ))}
            </div>
          )}
        </div>
      )}

      {selectedIds.size > 0 && (
        <div className="flex shrink-0 items-center gap-1.5 border-t border-border-subtle px-3 py-2 text-xs text-ink-muted">
          <span className="tabular-nums">{selectedIds.size} selected</span>
          <button
            onClick={() => archiveSelected()}
            className="ml-auto flex items-center gap-1.5 rounded-lg px-2 py-1.5 transition hover:bg-accent-subtle hover:text-ink"
            title="Archive selected"
          >
            <Archive className="size-3.5" /> Archive
          </button>
          <button
            onClick={() => deleteSelected()}
            className="flex items-center gap-1.5 rounded-lg px-2 py-1.5 transition hover:bg-danger/10 hover:text-danger"
            title="Delete selected"
          >
            <Trash2 className="size-3.5" /> Delete
          </button>
          <button
            onClick={() => setSelection(new Set())}
            aria-label="Clear selection"
            title="Clear selection"
            className="grid size-7 place-items-center rounded-lg text-ink-faint transition hover:bg-bg-hover hover:text-ink"
          >
            <X className="size-3.5" />
          </button>
        </div>
      )}

      <div className="shrink-0 border-t border-border-subtle px-2 py-1">
        <ProfileMenu variant="sidebar" />
      </div>

      {menu && (
        <ConversationContextMenu
          menu={{ x: menu.x, y: menu.y }}
          count={menu.ids.length}
          onClose={() => setMenu(null)}
          onArchive={() => {
            setSelection(new Set(menu.ids));
            archiveSelected();
          }}
          onDelete={() => {
            setSelection(new Set(menu.ids));
            deleteSelected();
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
