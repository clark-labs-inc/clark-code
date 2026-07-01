import { useMemo, useState } from "react";
import {
  Plus, MessageSquare, Trash2, PanelLeftClose, PanelLeft, FolderGit2, Server, Search, X,
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

function ConversationRow({ c, active }: { c: ConversationMeta; active: boolean }) {
  const open = useSessionStore((s) => s.openConversation);
  const remove = useSessionStore((s) => s.removeConversation);
  return (
    <div
      className={`group flex items-center gap-2 rounded-lg px-2.5 py-2 text-sm transition ${
        active ? "bg-bg-hover text-ink" : "text-ink-secondary hover:bg-bg-hover"
      }`}
    >
      <button
        onClick={() => void open(c.id)}
        className="flex min-w-0 flex-1 items-center gap-2 text-left"
        title={c.title}
      >
        <MessageSquare className="size-3.5 shrink-0 text-ink-faint" />
        <span className="flex min-w-0 flex-col">
          <span className="truncate leading-tight">{c.title}</span>
          <span className="truncate text-xs text-ink-muted">{relativeTime(c.updatedAt)}</span>
        </span>
      </button>
      <button
        onClick={() => remove(c.id)}
        title="Delete conversation"
        aria-label="Delete conversation"
        className="shrink-0 rounded-md p-1 text-ink-faint opacity-0 transition hover:bg-danger/10 hover:text-danger group-hover:opacity-100"
      >
        <Trash2 className="size-3.5" />
      </button>
    </div>
  );
}

export function Sidebar() {
  const collapsed = useSessionStore((s) => s.sidebarCollapsed);
  const setCollapsed = useSessionStore((s) => s.setSidebarCollapsed);
  const conversations = useSessionStore((s) => s.conversations);
  const session = useSessionStore((s) => s.session);
  const newConversation = useSessionStore((s) => s.endSession);
  const [filter, setFilter] = useState("");
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
  const groups = useMemo(() => groupByProject(visible), [visible]);

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
            Your conversations will show up here.
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
                    <ConversationRow key={c.id} c={c} active={session?.id === c.id} />
                  ))}
                </div>
              </section>
            ))}
          </div>
        )}
      </div>
    </aside>
  );
}
