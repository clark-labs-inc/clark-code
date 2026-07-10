import { useMemo, useState } from "react";
import { motion, useReducedMotion } from "motion/react";
import { ChevronRight, MessageSquare, FolderGit2, Server } from "lucide-react";
import { ClarkMark } from "./ClarkMark";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
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

/** The start screen — deliberately quiet: a "Welcome back" line and your recent
 *  sessions, nothing else. The composer + environment picker live below it
 *  (rendered by App), so a new session begins by simply typing a task. */
export function StartCard() {
  const auth = useSessionStore((s) => s.auth);
  const conversations = useSessionStore((s) => s.conversations);
  const conversationsLoading = useSessionStore((s) => s.conversationsLoading);
  const reduce = useReducedMotion();
  const [showAll, setShowAll] = useState(false);

  const firstName = (auth?.user.name ?? "").split(" ")[0] || auth?.user.name || "there";

  const recent = useMemo(
    () => conversations.filter((c) => !c.archived).sort((a, b) => b.updatedAt - a.updatedAt),
    [conversations],
  );
  const shown = showAll ? recent : recent.slice(0, 5);
  const hiddenCount = recent.length - shown.length;

  return (
    <div className="flex flex-1 flex-col overflow-y-auto">
      <motion.div
        initial={reduce ? false : { opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.25 }}
        className="mx-auto w-full max-w-2xl px-5 pb-6 pt-16"
      >
        <div className="mb-10 flex items-center gap-3">
          <ClarkMark size={28} className="shrink-0 rounded-xl" />
          <h1 className="text-2xl font-semibold tracking-tight text-ink">
            Welcome back, {firstName}
          </h1>
        </div>

        {recent.length > 0 ? (
          <div>
            <div className="mb-2.5 flex items-center justify-between">
              <h2 className="text-sm font-semibold text-ink-muted">Recent</h2>
              {hiddenCount > 0 && !showAll && (
                <button
                  onClick={() => setShowAll(true)}
                  className="text-sm font-medium text-ink-muted transition hover:text-ink"
                >
                  Show all
                </button>
              )}
              {showAll && recent.length > 5 && (
                <button
                  onClick={() => setShowAll(false)}
                  className="text-sm font-medium text-ink-muted transition hover:text-ink"
                >
                  Show less
                </button>
              )}
            </div>
            <div className="flex flex-col">
              {shown.map((c) => (
                <SessionRow key={c.id} c={c} />
              ))}
            </div>
          </div>
        ) : (
          <p className="text-sm text-ink-muted">
            {conversationsLoading
              ? "Loading your sessions…"
              : "Describe a task below to start your first session."}
          </p>
        )}
      </motion.div>
    </div>
  );
}

function SessionRow({ c }: { c: ConversationMeta }) {
  const open = useSessionStore((s) => s.openConversation);
  const Icon = c.remoteHost ? Server : c.project ? FolderGit2 : MessageSquare;
  const context = c.remoteHost ? c.remoteHost : c.project ? projectName(c.project) : null;

  return (
    <button
      onClick={() => void open(c.id)}
      className="group -mx-2 flex items-center gap-3 rounded-lg px-2 py-2 text-left transition hover:bg-bg-hover"
    >
      <Icon className="size-4 shrink-0 text-ink-faint" />
      <span className="min-w-0 flex-1 truncate text-sm text-ink">{c.title}</span>
      {context && (
        <span className="hidden shrink-0 truncate text-xs text-ink-faint sm:block">{context}</span>
      )}
      <span className="shrink-0 text-xs tabular-nums text-ink-faint">{relativeTime(c.updatedAt)}</span>
      <ChevronRight className="size-4 shrink-0 text-ink-faint opacity-0 transition group-hover:opacity-100" />
    </button>
  );
}
