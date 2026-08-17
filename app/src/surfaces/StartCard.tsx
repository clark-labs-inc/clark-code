import { useMemo, useState } from "react";
import { productName } from "../product/productModule";
import { useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { ChevronRight, MessageSquare, FolderGit2, Server } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import {
  DUR,
  RISE,
  accessibleMotion,
  staggeredTransition,
} from "../lib/motion";
import { isQuickChatProject, projectDisplayName } from "../lib/projectSidebar";
import { stableOrderIds } from "../lib/stableOrder";
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

function daypart(): string {
  const hour = new Date().getHours();
  if (hour < 12) return "Good morning";
  if (hour < 18) return "Good afternoon";
  return "Good evening";
}

/** The start screen — deliberately quiet: an editorial greeting and recent
 *  sessions, nothing else. The composer + environment picker live below it
 *  (rendered by App), so a new session begins by simply typing a task. */
export function StartCard() {
  const auth = useSessionStore((s) => s.auth);
  const conversations = useSessionStore((s) => s.conversations);
  const conversationsLoading = useSessionStore((s) => s.conversationsLoading);
  const reduce = useReducedMotion();
  const [showAll, setShowAll] = useState(false);

  const firstName = (auth?.user.name ?? "").split(" ")[0] || auth?.user.name || "there";

  // Creation time is the stable display key: the newest-created conversation
  // stays on top, while streamed `updatedAt` ticks cannot reshuffle the rows.
  const recent = useMemo(
    () => stableOrderIds(conversations.filter((c) => !c.archived)),
    [conversations],
  );
  const shown = showAll ? recent : recent.slice(0, 5);
  const hiddenCount = recent.length - shown.length;

  return (
    <div className="flex flex-1 flex-col overflow-y-auto">
      <m.div
        {...accessibleMotion(RISE, reduce)}
        transition={staggeredTransition(reduce, 0, 0.04, { duration: DUR.slow })}
        className="conversation-column-width mx-auto w-full px-6 pb-6 pt-10"
      >
        <div className="mb-6">
          <div className="mb-2.5 text-xs font-semibold uppercase tracking-[0.16em] text-accent">
            {productName()}
          </div>
          <h1 className="font-display max-w-xl text-3xl leading-[1.12] text-ink">
            {daypart()}, {firstName}.
          </h1>
          <p className="mt-2 max-w-lg text-base text-ink-muted">
            What should we build, investigate, or improve today?
          </p>
        </div>

        {recent.length > 0 ? (
          <div>
            <div className="mb-3 flex items-center justify-between px-1">
              <h2 className="text-xs font-semibold uppercase tracking-[0.12em] text-ink-muted">
                Recent work
              </h2>
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
      </m.div>
    </div>
  );
}

function SessionRow({ c }: { c: ConversationMeta }) {
  const open = useSessionStore((s) => s.openConversation);
  const quickChat = isQuickChatProject(c.project, c.id);
  const Icon = c.remoteHost ? Server : c.project && !quickChat ? FolderGit2 : MessageSquare;
  const context = c.remoteHost
    ? c.remoteHost
    : quickChat
      ? "Quick Chat"
      : c.project
        ? projectDisplayName(c.project)
        : null;

  return (
    <button
      onClick={() => void open(c.id)}
      className="group flex min-h-11 items-center gap-3 px-4 py-2.5 text-left transition duration-base ease-agent hover:bg-accent-subtle"
    >
      <Icon className="size-4 shrink-0 text-ink-faint" />
      <span className="min-w-0 flex-1 truncate text-sm text-ink">{c.title}</span>
      {context && (
        <span className="hidden shrink-0 truncate text-xs text-ink-faint sm:block">{context}</span>
      )}
      <span className="shrink-0 text-xs tabular-nums text-ink-faint">{relativeTime(c.updatedAt)}</span>
      <ChevronRight className="size-4 shrink-0 text-ink-faint opacity-0 transition duration-base group-hover:translate-x-0.5 group-hover:text-accent group-hover:opacity-100" />
    </button>
  );
}
