import { useState } from "react";
import { productName } from "../product/productModule";
import { Sun, Moon, FolderGit2, SquareTerminal, Settings as SettingsIcon, RefreshCw, Share2 } from "lucide-react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { useSessionStore } from "../store/sessionStore";
import { projectDisplayName } from "../lib/projectSidebar";
import { cn } from "../lib/cn";
import { FADE, accessibleMotion } from "../lib/motion";
import { ChangesButton } from "./ChangesPanel";
import { MemoryButton } from "./MemoryPanel";
import { SecurityButton } from "./SecurityPanel";

/** Update affordance in the top bar. While a new version downloads in the
 *  background it shows live progress; once staged it becomes a non-blocking
 *  "Ready to update" button that relaunches into the new binary. */
export function UpdatePill() {
  const update = useSessionStore((s) => s.update);
  const progress = useSessionStore((s) => s.updateProgress);
  const apply = useSessionStore((s) => s.applyUpdate);
  const waiting = useSessionStore((s) => s.updateWaiting);
  const reduce = useReducedMotion();

  return (
    <UpdatePillView
      update={update}
      progress={progress}
      waiting={waiting}
      reduce={reduce}
      onApply={apply}
    />
  );
}

export function UpdatePillView({
  update,
  progress,
  waiting,
  reduce,
  onApply,
}: {
  update: { version: string } | null;
  progress: { downloaded: number; total: number | null } | null;
  waiting: boolean;
  reduce: boolean | null;
  onApply: () => Promise<void>;
}) {
  const content = progress ? (
    <DownloadingPill progress={progress} />
  ) : update && waiting ? (
    <div
      key="waiting"
      className="flex items-center gap-1.5 rounded-xl bg-accent-soft px-3 py-1.5 text-xs font-semibold text-accent"
      title="The update will install after active runs and queued follow-ups finish"
    >
      <RefreshCw className="size-3.5 animate-[spin_1.4s_linear_infinite]" />
      Finishing work before update…
    </div>
  ) : update ? (
    <button
      key="restart"
      onClick={() => void onApply()}
      aria-label={`Ready to update ${productName()} to ${update.version}; restart now`}
      title={`${productName()} ${update.version} is ready — relaunch to update`}
      className="flex shrink-0 items-center gap-1.5 rounded-xl bg-accent-soft px-3 py-1.5 text-xs font-semibold text-accent transition duration-base ease-agent hover:bg-accent/20"
    >
      <RefreshCw className="size-3.5" />
      Ready to update
    </button>
  ) : null;

  return (
    <AnimatePresence mode="wait" initial={false}>
      {content && (
        <m.div
          key={progress ? "downloading" : waiting ? "waiting" : "restart"}
          {...accessibleMotion(FADE, reduce)}
        >
          {content}
        </m.div>
      )}
    </AnimatePresence>
  );
}

function DownloadingPill({ progress }: { progress: { downloaded: number; total: number | null } }) {
  const pct =
    progress.total && progress.total > 0
      ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100))
      : null;
  return (
    <div
      className="flex items-center gap-2 rounded-xl bg-accent-soft px-3 py-1.5 text-xs font-semibold text-accent"
      title={`Downloading the latest ${productName()}…`}
    >
      <RefreshCw className="size-3.5 animate-[spin_1.4s_linear_infinite]" />
      <span className="tabular-nums">
        {pct !== null ? `Downloading update ${pct}%` : "Downloading update…"}
      </span>
      <span className="h-1 w-12 overflow-hidden rounded-full bg-accent/20">
        <span
          className={cn(
            "block h-full rounded-full bg-accent",
            pct !== null ? "transition-[width] duration-slow" : "w-1/3 animate-pulse",
          )}
          style={pct !== null ? { width: `${Math.max(4, pct)}%` } : undefined}
        />
      </span>
    </div>
  );
}

/** Share = a network roundtrip (mint link + copy). Spinner + disabled while it
 *  runs so the click is acknowledged and can't double-fire. */
function ShareButton({ onShare }: { onShare: () => Promise<void> }) {
  const [sharing, setSharing] = useState(false);
  return (
    <button
      onClick={() => {
        setSharing(true);
        void onShare().finally(() => setSharing(false));
      }}
      disabled={sharing}
      aria-label="Share conversation"
      title="Copy a public read-only link (/unshare stops sharing)"
      className="grid size-9 place-items-center rounded-xl text-ink-muted transition duration-base ease-agent hover:bg-accent-subtle hover:text-accent disabled:opacity-60"
    >
      {sharing ? (
        <RefreshCw className="size-4 animate-[spin_1s_linear_infinite]" />
      ) : (
        <Share2 className="size-4" />
      )}
    </button>
  );
}

export function TopBar({ dark, onToggleTheme }: { dark: boolean; onToggleTheme: () => void }) {
  const session = useSessionStore((s) => s.session);
  const connecting = useSessionStore((s) => s.connecting);
  const terminalOpen = useSessionStore((s) => s.terminalOpen);
  const toggleTerminal = useSessionStore((s) => s.toggleTerminal);
  const setSettingsOpen = useSessionStore((s) => s.setSettingsOpen);
  const shareConversation = useSessionStore((s) => s.shareConversation);
  const signedIn = useSessionStore((s) => s.auth !== null);
  const title = useSessionStore((s) =>
    session ? s.conversations.find((c) => c.id === session.id)?.title : null,
  );
  const projectCwd = useSessionStore((s) => s.activeProjectRoot ?? s.localSettings.cwd);
  const activeRemote = useSessionStore((s) => s.activeRemote);
  const isLocal = session?.provider === "local";

  return (
    <header className="flex h-12 shrink-0 items-center gap-3 bg-bg px-5">
      {session && (
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <span
            className={`size-1.5 shrink-0 rounded-full ${connecting ? "animate-pulse bg-warning" : "bg-success"}`}
            title={connecting ? "Reconnecting in background" : "Connected"}
            aria-label={connecting ? "Reconnecting in background" : "Connected"}
          />
          {isLocal && projectCwd && (
            <span
              title={projectCwd}
              className="hidden shrink-0 items-center gap-1.5 rounded-lg bg-bg-secondary px-2.5 py-1 text-xs font-medium text-ink-secondary md:flex"
            >
              <FolderGit2 className="size-3" />
              {projectDisplayName(projectCwd)}
            </span>
          )}
          <span className="truncate text-sm font-semibold text-ink-secondary">
            {title ?? "New conversation"}
          </span>
        </div>
      )}

      <div className="ml-auto flex items-center gap-1">
        <UpdatePill />
        <span className="hidden sm:contents">
          {session && signedIn && <ShareButton onShare={shareConversation} />}
          {session && isLocal && <ChangesButton />}
          {session && isLocal && !activeRemote && projectCwd && <SecurityButton />}
          {session && isLocal && projectCwd && <MemoryButton />}
        </span>
        <button
          onClick={() => setSettingsOpen(true)}
          aria-label="Settings"
          title="Settings (⌘,)"
          className="grid size-9 place-items-center rounded-xl text-ink-muted transition duration-base ease-agent hover:bg-accent-subtle hover:text-accent"
        >
          <SettingsIcon className="size-4" />
        </button>
        {session && !activeRemote && (
          <button
            onClick={toggleTerminal}
            aria-label={terminalOpen ? "Hide terminal" : "Show terminal"}
            title="Terminal (run commands in your project)"
            className={cn(
              "grid size-9 place-items-center rounded-xl transition duration-base ease-agent",
              terminalOpen
                ? "bg-accent-soft text-accent"
                : "text-ink-muted hover:bg-accent-subtle hover:text-accent",
            )}
          >
            <SquareTerminal className="size-4" />
          </button>
        )}
        <button
          onClick={onToggleTheme}
          aria-label={dark ? "Switch to light theme" : "Switch to dark theme"}
          className="hidden size-9 place-items-center rounded-xl text-ink-muted transition duration-base ease-agent hover:bg-accent-subtle hover:text-accent sm:grid"
        >
          {dark ? <Sun className="size-4" /> : <Moon className="size-4" />}
        </button>
      </div>
    </header>
  );
}
