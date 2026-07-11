import { useEffect, useMemo, useRef, useState } from "react";
import {
  Plus, MessageSquare, Archive, ArchiveRestore, ChevronRight, PanelLeftClose, PanelLeft,
  FolderGit2, Server, Search, X, Trash2, Loader2,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
import { useIsNarrow } from "../lib/responsive";
import { fuzzyFilter } from "../lib/fuzzy";
import { cn } from "../lib/cn";
import { ProfileMenu } from "./ProfileMenu";
import type { ConversationMeta } from "../lib/history";

function relativeTime(ts: number): string {
  const s = Math.max(0, (Date.now() - ts) / 1000);
  if (s < 60) return "just now";
  const m = s / 60;
  if (m < 60) return `${Math.floor(m)}m ago`;
  const h = m / 60;
  if (h < 24) return `${Math.floor(h)}h ago`;
  const d = h / 24;
  if (d < 7) return `${Math.floor(d)}d ago`;
  return new Date(ts).toLocaleDateString();
}

type GroupKind = "remote" | "local" | "none";
interface ProjectGroup {
  key: string;
  label: string;
  title: string; // full path / host for the tooltip
  kind: GroupKind;
  convos: ConversationMeta[];
  latest: number;
}

/** Group conversations by their project (remote host, local folder, or none),
 *  Codex-style: newest project first, newest conversation first within each. */
function groupByProject(list: ConversationMeta[]): ProjectGroup[] {
  const map = new Map<string, ProjectGroup>();
  for (const c of list) {
    let key: string, label: string, title: string, kind: GroupKind;
    if (c.remoteHost) {
      key = `r:${c.remoteHost}`;
      label = c.remoteHost;
      title = `Remote · ${c.remoteHost}${c.project ? ` · ${c.project}` : ""}`;
      kind = "remote";
    } else if (c.project) {
      key = `p:${c.project}`;
      label = projectName(c.project);
      title = c.project;
      kind = "local";
    } else {
      key = "none";
      label = "Other";
      title = "Conversations without a project";
      kind = "none";
    }
    let g = map.get(key);
    if (!g) {
      g = { key, label, title, kind, convos: [], latest: 0 };
      map.set(key, g);
    }
    g.convos.push(c);
    g.latest = Math.max(g.latest, c.updatedAt);
  }
  const groups = [...map.values()];
  for (const g of groups) g.convos.sort((a, b) => b.updatedAt - a.updatedAt);
  groups.sort((a, b) => b.latest - a.latest);
  return groups;
}

function GroupHeader({ group }: { group: ProjectGroup }) {
  const Icon = group.kind === "remote" ? Server : group.kind === "local" ? FolderGit2 : MessageSquare;
  return (
    <div
      title={group.title}
      className="mb-1 mt-3 flex items-center gap-1.5 px-2 text-xs font-semibold uppercase tracking-[0.12em] text-ink-faint first:mt-0.5"
    >
      <Icon className={`size-3 shrink-0 ${group.kind === "remote" ? "text-accent" : ""}`} />
      <span className="truncate">{group.label}</span>
      <span className="ml-auto shrink-0 font-mono text-xs font-normal tracking-normal text-ink-faint/70">
        {group.convos.length}
      </span>
    </div>
  );
}

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
        "group relative flex items-center gap-2 rounded-xl px-3 py-1.5 text-sm transition duration-200 ease-clark",
        selected
          ? "bg-accent-soft text-ink ring-1 ring-accent/40"
          : active || opening
            ? "bg-accent-soft text-ink ring-1 ring-accent/10"
            : "text-ink-secondary hover:bg-accent-subtle hover:text-ink",
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
          className="flex min-w-0 flex-1 items-center gap-2 text-left"
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
            <span className="grid size-3.5 shrink-0 place-items-center" aria-label="Selected">
              <span className="size-2 rounded-sm bg-accent" />
            </span>
          ) : (
            <MessageSquare
              className={`size-3.5 shrink-0 ${active || opening ? "text-accent" : "text-ink-faint"}`}
            />
          )}
          <span className="flex min-w-0 flex-col">
            <span className="truncate leading-tight">{c.title}</span>
            <span className="truncate text-xs text-ink-muted">
              {opening ? "Opening…" : streaming ? "Working…" : relativeTime(c.updatedAt)}
            </span>
          </span>
        </button>
      )}
      {!editing && (
        <button
          onClick={() => archive(c.id)}
          title="Archive conversation"
          aria-label="Archive conversation"
          className="shrink-0 rounded-md p-1 text-ink-faint opacity-0 transition hover:bg-bg-sunken hover:text-ink group-hover:opacity-100"
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
    <div className="group flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-sm text-ink-faint transition hover:bg-bg-hover">
      <button
        onClick={() => restore(c.id)}
        title={`Restore “${c.title}”`}
        aria-label={`Restore ${c.title}`}
        className="flex min-w-0 flex-1 items-center gap-2 text-left transition hover:text-ink-secondary"
      >
        <MessageSquare className="size-3.5 shrink-0 text-ink-faint" />
        <span className="min-w-0 flex-1 truncate leading-tight">{c.title}</span>
        <ArchiveRestore className="size-3.5 shrink-0 opacity-0 transition group-hover:opacity-100" />
      </button>
      {confirming ? (
        <span className="flex shrink-0 items-center gap-1">
          <button
            onClick={() => del(c.id)}
            aria-label={`Permanently delete ${c.title}`}
            className="min-h-8 rounded-md px-2 text-xs font-medium text-danger transition hover:bg-danger/10"
          >
            Delete
          </button>
          <button
            onClick={() => setConfirming(false)}
            aria-label="Cancel delete"
            className="min-h-8 rounded-md px-2 text-xs text-ink-muted transition hover:bg-bg-hover hover:text-ink"
          >
            Cancel
          </button>
        </span>
      ) : (
        <button
          onClick={() => setConfirming(true)}
          title="Delete permanently"
          aria-label={`Delete ${c.title} permanently`}
          className="grid size-8 shrink-0 place-items-center rounded-md text-ink-faint opacity-0 transition hover:bg-danger/10 hover:text-danger group-hover:opacity-100"
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

export function Sidebar() {
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
  const selectedIds = useSessionStore((s) => s.selectedConversationIds);
  const setSelection = useSessionStore((s) => s.setConversationSelection);
  const archiveSelected = useSessionStore((s) => s.archiveSelectedConversations);
  const deleteSelected = useSessionStore((s) => s.deleteSelectedConversations);
  const [filter, setFilter] = useState("");
  const [archivedOpen, setArchivedOpen] = useState(false);
  // The right-click menu: positioned at the cursor; acts on the selection when
  // the right-clicked row is in it, else just that row.
  const [menu, setMenu] = useState<{ x: number; y: number; ids: string[] } | null>(null);
  // Below this width the full sidebar would crowd out the conversation, so it
  // auto-collapses to the icon rail (and can't be expanded until there's room).
  const narrow = useIsNarrow(768);

  const visible = useMemo(
    () =>
      fuzzyFilter(
        conversations,
        filter,
        (c) => `${c.title} ${c.project ? projectName(c.project) : ""} ${c.remoteHost ?? ""}`,
        200,
      ).map((m) => m.item),
    [conversations, filter],
  );
  // Archived conversations are hidden from the project groups and collected into
  // their own collapsed section (search still matches across both).
  const activeConvos = useMemo(() => visible.filter((c) => !c.archived), [visible]);
  const archivedConvos = useMemo(
    () => visible.filter((c) => c.archived).sort((a, b) => b.updatedAt - a.updatedAt),
    [visible],
  );
  const groups = useMemo(() => groupByProject(activeConvos), [activeConvos]);

  const openContextMenu = (e: React.MouseEvent, id: string) => {
    e.preventDefault();
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
      <div className="flex w-14 shrink-0 flex-col items-center gap-2 border-r border-border-subtle bg-bg-elevated py-3">
        {!narrow && (
          <button
            onClick={() => setCollapsed(false)}
            aria-label="Expand sidebar"
            className="grid size-9 place-items-center rounded-xl text-ink-muted transition duration-200 ease-clark hover:bg-accent-subtle hover:text-accent"
          >
            <PanelLeft className="size-4" />
          </button>
        )}
        <button
          onClick={() => newConversation()}
          aria-label="New session"
          title="New session"
          className="grid size-9 place-items-center rounded-xl bg-accent text-on-accent shadow-soft transition duration-200 ease-clark hover:-translate-y-0.5 hover:bg-accent-hover"
        >
          <Plus className="size-4" />
        </button>
        <div className="mt-auto">
          <ProfileMenu />
        </div>
      </div>
    );
  }

  return (
    <aside className="flex w-[17.5rem] shrink-0 flex-col border-r border-border-subtle bg-bg-elevated shadow-[10px_0_30px_rgba(55,48,42,0.025)]">
      <div className="flex h-14 shrink-0 items-center gap-2 px-4">
        <span className="font-display text-[1.65rem] leading-none text-ink">clark</span>
        <span className="rounded-full bg-accent-soft px-2 py-0.5 text-[0.6875rem] font-semibold uppercase tracking-[0.12em] text-accent">
          Code
        </span>
        <button
          onClick={() => setCollapsed(true)}
          aria-label="Collapse sidebar"
          className="ml-auto grid size-8 place-items-center rounded-xl text-ink-faint transition duration-200 ease-clark hover:bg-accent-subtle hover:text-accent"
        >
          <PanelLeftClose className="size-4" />
        </button>
      </div>

      <div className="px-3 pb-2">
        <button
          onClick={() => newConversation()}
          className="flex w-full items-center gap-2.5 rounded-xl bg-[linear-gradient(135deg,var(--color-accent-gradient-start),var(--color-accent-gradient-end))] px-3.5 py-2.5 text-sm font-semibold text-on-accent shadow-soft transition duration-200 ease-clark hover:-translate-y-0.5 hover:brightness-[0.96] hover:shadow-lifted active:translate-y-0"
        >
          <Plus className="size-4" /> New session
        </button>
      </div>

      <div className="px-3 pb-2.5">
        <div className="flex items-center gap-2.5 rounded-xl bg-bg-secondary px-3 py-2 ring-1 ring-transparent transition duration-200 focus-within:bg-bg-elevated focus-within:ring-accent/25">
          <Search className="size-3.5 shrink-0 text-ink-faint" />
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Search conversations…"
            aria-label="Search conversations"
            className="composer-input min-w-0 flex-1 bg-transparent text-xs text-ink outline-none placeholder:text-ink-faint"
          />
          {filter && (
            <button
              onClick={() => setFilter("")}
              aria-label="Clear search"
              className="grid size-8 shrink-0 place-items-center rounded-full text-ink-faint transition hover:bg-accent-soft hover:text-accent"
            >
              <X className="size-3" />
            </button>
          )}
        </div>
      </div>

      <div
        className="min-h-0 flex-1 overflow-y-auto px-3 pb-4"
        onClick={(e) => {
          // A plain click on empty list space (not on a row) clears the
          // Shift-click selection.
          if (e.target === e.currentTarget && selectedIds.size > 0) setSelection(new Set());
        }}
      >
        {conversations.length === 0 ? (
          <p className="px-1 py-6 text-center text-xs text-ink-faint">
            {conversationsLoading ? "Loading conversations…" : "Your conversations will show up here."}
          </p>
        ) : visible.length === 0 ? (
          <p className="px-1 py-6 text-center text-xs text-ink-faint">
            No conversations match “{filter}”.
          </p>
        ) : (
          <div className="flex flex-col">
            {groups.map((g) => (
              <section key={g.key}>
                <GroupHeader group={g} />
                <div className="flex flex-col gap-0.5">
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

            {archivedConvos.length > 0 && (
              <section>
                <button
                  onClick={() => setArchivedOpen((o) => !o)}
                  aria-expanded={archivedOpen}
                  className="mt-3 mb-1 flex w-full items-center gap-1.5 px-1.5 text-xs font-semibold uppercase tracking-wider text-ink-faint transition hover:text-ink-muted first:mt-0.5"
                >
                  <ChevronRight
                    className={`size-3 shrink-0 transition-transform ${archivedOpen ? "rotate-90" : ""}`}
                  />
                  <Archive className="size-3 shrink-0" />
                  <span>Archived</span>
                  <span className="ml-auto shrink-0 font-mono text-xs font-normal tracking-normal text-ink-faint/70">
                    {archivedConvos.length}
                  </span>
                </button>
                {archivedOpen && (
                  <div className="flex flex-col gap-0.5">
                    {archivedConvos.map((c) => (
                      <ArchivedRow key={c.id} c={c} />
                    ))}
                  </div>
                )}
              </section>
            )}
          </div>
        )}
      </div>

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

      <div className="shrink-0 border-t border-border-subtle p-2">
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
    </aside>
  );
}
