import { useMemo, useState } from "react";
import {
  Plus, MessageSquare, Archive, ArchiveRestore, ChevronRight, PanelLeftClose, PanelLeft,
  FolderGit2, Server, Search, X, Trash2,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
import { useIsNarrow } from "../lib/responsive";
import { fuzzyFilter } from "../lib/fuzzy";
import { ClarkMark } from "./ClarkMark";
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
      className="mt-3 mb-1 flex items-center gap-1.5 px-1.5 text-[0.68rem] font-semibold uppercase tracking-wider text-ink-faint first:mt-0.5"
    >
      <Icon className={`size-3 shrink-0 ${group.kind === "remote" ? "text-accent" : ""}`} />
      <span className="truncate">{group.label}</span>
      <span className="ml-auto shrink-0 font-mono text-[0.62rem] font-normal tracking-normal text-ink-faint/70">
        {group.convos.length}
      </span>
    </div>
  );
}

function ConversationRow({
  c,
  active,
  streaming,
}: {
  c: ConversationMeta;
  active: boolean;
  /** A run is currently streaming in this conversation. */
  streaming: boolean;
}) {
  const open = useSessionStore((s) => s.openConversation);
  const archive = useSessionStore((s) => s.archiveConversation);
  const rename = useSessionStore((s) => s.renameConversation);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(c.title);

  const commit = () => {
    setEditing(false);
    rename(c.id, draft);
  };

  return (
    <div
      className={`group relative flex items-center gap-2 rounded-lg px-2.5 py-2 text-sm transition ${
        active
          ? "bg-bg-hover text-ink before:absolute before:left-0 before:top-1/2 before:h-5 before:w-0.5 before:-translate-y-1/2 before:rounded-full before:bg-accent before:content-['']"
          : "text-ink-secondary hover:bg-bg-hover"
      }`}
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
          onClick={() => void open(c.id)}
          onDoubleClick={() => {
            setDraft(c.title);
            setEditing(true);
          }}
          className="flex min-w-0 flex-1 items-center gap-2 text-left"
          title={`${c.title} — double-click to rename`}
        >
          {streaming ? (
            <span className="relative grid size-3.5 shrink-0 place-items-center" aria-label="Running">
              <span className="absolute size-2 animate-ping rounded-full bg-accent/40" />
              <span className="size-1.5 rounded-full bg-accent" />
            </span>
          ) : (
            <MessageSquare className="size-3.5 shrink-0 text-ink-faint" />
          )}
          <span className="flex min-w-0 flex-col">
            <span className="truncate leading-tight">{c.title}</span>
            <span className="truncate text-xs text-ink-muted">
              {streaming ? "Working…" : relativeTime(c.updatedAt)}
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
            className="rounded px-1.5 py-0.5 text-[11px] font-medium text-danger transition hover:bg-danger/10"
          >
            Delete
          </button>
          <button
            onClick={() => setConfirming(false)}
            aria-label="Cancel delete"
            className="rounded px-1 py-0.5 text-[11px] text-ink-muted transition hover:text-ink"
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

export function Sidebar() {
  const collapsed = useSessionStore((s) => s.sidebarCollapsed);
  const setCollapsed = useSessionStore((s) => s.setSidebarCollapsed);
  const conversations = useSessionStore((s) => s.conversations);
  const conversationsLoading = useSessionStore((s) => s.conversationsLoading);
  const session = useSessionStore((s) => s.session);
  // Selection follows what's on screen (a peek shows another conversation while
  // the live one keeps streaming — that one gets the pulsing "working" dot).
  const peekId = useSessionStore((s) => s.peek?.id ?? null);
  const liveBusy = useSessionStore((s) =>
    Object.values(s.snapshot.runs).some((r) => r.status === "running" || r.status === "queued"),
  );
  const newConversation = useSessionStore((s) => s.endSession);
  const [filter, setFilter] = useState("");
  const [archivedOpen, setArchivedOpen] = useState(false);
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

  if (collapsed || narrow) {
    return (
      <div className="flex w-12 shrink-0 flex-col items-center gap-2 border-r border-border bg-bg-elevated/40 py-3">
        {!narrow && (
          <button
            onClick={() => setCollapsed(false)}
            aria-label="Expand sidebar"
            className="grid size-8 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover"
          >
            <PanelLeft className="size-4" />
          </button>
        )}
        <button
          onClick={() => newConversation()}
          aria-label="New conversation"
          title="New conversation"
          className="grid size-8 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover"
        >
          <Plus className="size-4" />
        </button>
      </div>
    );
  }

  return (
    <aside className="flex w-64 shrink-0 flex-col border-r border-border bg-bg-elevated/40">
      <div className="flex h-12 shrink-0 items-center gap-2 px-3">
        <ClarkMark size={18} className="rounded-[5px]" />
        <span className="text-sm font-semibold tracking-tight text-ink">Clark</span>
        <button
          onClick={() => setCollapsed(true)}
          aria-label="Collapse sidebar"
          className="ml-auto grid size-7 place-items-center rounded-lg text-ink-faint transition hover:bg-bg-hover"
        >
          <PanelLeftClose className="size-4" />
        </button>
      </div>

      <div className="px-2.5 pb-2">
        <button
          onClick={() => newConversation()}
          className="flex w-full items-center gap-2 rounded-lg border border-border-subtle bg-bg-elevated/70 px-2.5 py-2 text-sm font-medium text-ink-secondary transition hover:bg-bg-hover"
        >
          <Plus className="size-4" /> New conversation
        </button>
      </div>

      {conversations.length > 4 && (
        <div className="px-2.5 pb-2">
          <div className="flex items-center gap-2 rounded-lg bg-bg-sunken px-2.5 py-1.5 ring-1 ring-transparent focus-within:ring-border-subtle">
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
                className="grid size-4 shrink-0 place-items-center rounded-full text-ink-faint transition hover:text-ink"
              >
                <X className="size-3" />
              </button>
            )}
          </div>
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto px-2.5 pb-3">
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
                      active={(peekId ?? session?.id) === c.id}
                      streaming={liveBusy && session?.id === c.id}
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
                  className="mt-3 mb-1 flex w-full items-center gap-1.5 px-1.5 text-[0.68rem] font-semibold uppercase tracking-wider text-ink-faint transition hover:text-ink-muted first:mt-0.5"
                >
                  <ChevronRight
                    className={`size-3 shrink-0 transition-transform ${archivedOpen ? "rotate-90" : ""}`}
                  />
                  <Archive className="size-3 shrink-0" />
                  <span>Archived</span>
                  <span className="ml-auto shrink-0 font-mono text-[0.62rem] font-normal tracking-normal text-ink-faint/70">
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
    </aside>
  );
}
