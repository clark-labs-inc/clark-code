import { useMemo, useState } from "react";
import { motion, useReducedMotion } from "motion/react";
import { ArrowRight, ChevronRight, MessageSquare, FolderGit2, Server } from "lucide-react";
import { ClarkMark } from "./ClarkMark";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
import { useAppVersion } from "../lib/appInfo";
import type { ConversationMeta } from "../lib/history";

const SAMPLES = [
  "In one sentence, what is the Rust programming language?",
  "Build a one-page website about cats and publish it. Give me the URL.",
  "Research the best way to add authentication, then implement it.",
];

const LOCAL_SAMPLES = [
  "Summarize what this project does from its README and top-level files.",
  "Find every TODO in the codebase and list them by file.",
  "Add a unit test for the function in the file I'm about to mention.",
];

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

/** The start screen: a "Welcome back" header + the recent-sessions list. The
 *  composer + environment picker live below it (rendered by App), so a new
 *  session begins by simply typing a task. */
export function StartCard() {
  const auth = useSessionStore((s) => s.auth);
  const conversations = useSessionStore((s) => s.conversations);
  const conversationsLoading = useSessionStore((s) => s.conversationsLoading);
  const activeProvider = useSessionStore((s) => s.activeProvider);
  const setPrefill = useSessionStore((s) => s.setComposerPrefill);
  const version = useAppVersion();
  const reduce = useReducedMotion();
  const [showAll, setShowAll] = useState(false);

  const isLocal = activeProvider === "local";
  const samples = isLocal ? LOCAL_SAMPLES : SAMPLES;
  const firstName = (auth?.user.name ?? "").split(" ")[0] || auth?.user.name || "there";

  const recent = useMemo(
    () => conversations.filter((c) => !c.archived).sort((a, b) => b.updatedAt - a.updatedAt),
    [conversations],
  );
  const shown = showAll ? recent : recent.slice(0, 4);
  const hiddenCount = recent.length - shown.length;

  return (
    <div className="flex flex-1 flex-col overflow-y-auto">
      <motion.div
        initial={reduce ? false : { opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.25 }}
        className="mx-auto w-full max-w-3xl px-5 pb-4 pt-10"
      >
        {/* Header */}
        <div className="mb-8 flex items-center gap-3">
          <ClarkMark size={30} className="shrink-0 rounded-xl" />
          <h1 className="flex-1 text-2xl font-semibold tracking-tight text-ink">
            Welcome back, {firstName}
          </h1>
          {version && (
            <span className="shrink-0 font-mono text-xs tabular-nums text-ink-faint">v{version}</span>
          )}
        </div>

        {/* Sessions */}
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-sm font-semibold uppercase tracking-wider text-ink-muted">Sessions</h2>
          {hiddenCount > 0 && !showAll && (
            <button
              onClick={() => setShowAll(true)}
              className="text-sm font-medium text-ink-muted transition hover:text-ink"
            >
              Show {hiddenCount} more
            </button>
          )}
          {showAll && recent.length > 4 && (
            <button
              onClick={() => setShowAll(false)}
              className="text-sm font-medium text-ink-muted transition hover:text-ink"
            >
              Show less
            </button>
          )}
        </div>

        {recent.length === 0 ? (
          <div className="rounded-xl border border-dashed border-border bg-bg-elevated/40 px-5 py-8 text-center">
            <p className="text-sm text-ink-muted">
              {conversationsLoading
                ? "Loading your sessions…"
                : "No sessions yet — describe a task below to begin."}
            </p>
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            {shown.map((c) => (
              <SessionRow key={c.id} c={c} />
            ))}
          </div>
        )}

        {/* Try */}
        <div className="mt-8">
          <h2 className="mb-3 text-sm font-semibold uppercase tracking-wider text-ink-muted">
            Try one of these
          </h2>
          <div className="flex flex-col gap-1">
            {samples.map((s) => (
              <button
                key={s}
                onClick={() => setPrefill(s)}
                className="group flex items-center gap-2.5 rounded-lg px-2 py-2 text-left text-sm text-ink-secondary transition hover:bg-bg-hover hover:text-ink"
              >
                <ArrowRight className="size-4 shrink-0 text-ink-faint transition group-hover:translate-x-0.5 group-hover:text-ink-muted" />
                <span className="truncate">{s}</span>
              </button>
            ))}
          </div>
        </div>
      </motion.div>
    </div>
  );
}

function SessionRow({ c }: { c: ConversationMeta }) {
  const open = useSessionStore((s) => s.openConversation);
  const Icon = c.remoteHost ? Server : c.project ? FolderGit2 : MessageSquare;
  const context = c.remoteHost
    ? c.remoteHost
    : c.project
      ? projectName(c.project)
      : "No project";

  return (
    <button
      onClick={() => void open(c.id)}
      className="group flex items-center gap-3 rounded-xl border border-border-subtle bg-bg-elevated/50 px-4 py-3 text-left transition hover:border-border hover:bg-bg-hover"
    >
      <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-bg-sunken text-ink-muted">
        <Icon className="size-4" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-ink">{c.title}</span>
        <span className="block truncate text-xs text-ink-muted">{context}</span>
      </span>
      <span className="shrink-0 text-xs tabular-nums text-ink-faint">{relativeTime(c.updatedAt)}</span>
      <ChevronRight className="size-4 shrink-0 text-ink-faint transition group-hover:translate-x-0.5 group-hover:text-ink-muted" />
    </button>
  );
}
