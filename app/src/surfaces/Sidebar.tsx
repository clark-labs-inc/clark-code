import { useState } from "react";
import { motion, AnimatePresence } from "motion/react";
import {
  Plus, MessageSquare, Trash2, PanelLeftClose, PanelLeft, FolderGit2,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
import { useIsNarrow } from "../lib/responsive";
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
          <span className="flex min-w-0 items-center gap-1 truncate text-xs text-ink-muted">
            {c.project && (
              <>
                <FolderGit2 className="size-3 shrink-0" />
                <span className="truncate">{projectName(c.project)}</span>
                <span className="shrink-0 text-ink-faint">·</span>
              </>
            )}
            <span className="shrink-0">{relativeTime(c.updatedAt)}</span>
          </span>
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
  const [collapsed, setCollapsed] = useState(false);
  const conversations = useSessionStore((s) => s.conversations);
  const session = useSessionStore((s) => s.session);
  const newConversation = useSessionStore((s) => s.endSession);
  // Below this width the full sidebar would crowd out the conversation, so it
  // auto-collapses to the icon rail (and can't be expanded until there's room).
  const narrow = useIsNarrow(768);

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

      <div className="min-h-0 flex-1 overflow-y-auto px-2.5 pb-3">
        {conversations.length === 0 ? (
          <p className="px-1 py-6 text-center text-xs text-ink-faint">
            Your conversations will show up here.
          </p>
        ) : (
          <div className="flex flex-col gap-0.5">
            <AnimatePresence initial={false}>
              {conversations.map((c) => (
                <motion.div
                  key={c.id}
                  layout
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0, height: 0 }}
                >
                  <ConversationRow c={c} active={session?.id === c.id} />
                </motion.div>
              ))}
            </AnimatePresence>
          </div>
        )}
      </div>
    </aside>
  );
}
