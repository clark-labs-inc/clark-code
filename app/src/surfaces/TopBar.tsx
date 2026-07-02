import { Sun, Moon, FolderGit2, SquareTerminal, Settings as SettingsIcon, RefreshCw, Share2 } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
import { cn } from "../lib/cn";
import { ChangesButton } from "./ChangesPanel";
import { MemoryButton } from "./MemoryPanel";
import { ProfileMenu } from "./ProfileMenu";

/** Update affordance in the top bar. While a new version downloads in the
 *  background it shows live progress; once staged it becomes a non-blocking
 *  "Restart to update" button that relaunches into the new binary. */
function UpdatePill() {
  const update = useSessionStore((s) => s.update);
  const progress = useSessionStore((s) => s.updateProgress);
  const apply = useSessionStore((s) => s.applyUpdate);

  const content = progress ? (
    <DownloadingPill progress={progress} />
  ) : update ? (
    <button
      key="restart"
      onClick={() => void apply()}
      title={`Clark Code ${update.version} is ready — relaunch to update`}
      className="flex items-center gap-1.5 rounded-lg bg-accent/15 px-2.5 py-1 text-xs font-medium text-accent transition hover:bg-accent/25"
    >
      <RefreshCw className="size-3.5" />
      Restart to update
    </button>
  ) : null;

  return (
    <AnimatePresence mode="wait" initial={false}>
      {content && (
        <motion.div
          key={progress ? "downloading" : "restart"}
          initial={{ opacity: 0, y: -3 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -3 }}
          transition={{ duration: 0.18, ease: [0.4, 0, 0.2, 1] }}
        >
          {content}
        </motion.div>
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
      className="flex items-center gap-2 rounded-lg bg-accent/10 px-2.5 py-1 text-xs font-medium text-accent"
      title="Downloading the latest Clark Code…"
    >
      <RefreshCw className="size-3.5 animate-[spin_1.4s_linear_infinite]" />
      <span className="tabular-nums">
        {pct !== null ? `Downloading update ${pct}%` : "Downloading update…"}
      </span>
      <span className="h-1 w-12 overflow-hidden rounded-full bg-accent/20">
        <span
          className={cn(
            "block h-full rounded-full bg-accent",
            pct !== null ? "transition-[width] duration-300" : "w-1/3 animate-pulse",
          )}
          style={pct !== null ? { width: `${Math.max(4, pct)}%` } : undefined}
        />
      </span>
    </div>
  );
}

export function TopBar({ dark, onToggleTheme }: { dark: boolean; onToggleTheme: () => void }) {
  const session = useSessionStore((s) => s.session);
  const auth = useSessionStore((s) => s.auth);
  const terminalOpen = useSessionStore((s) => s.terminalOpen);
  const toggleTerminal = useSessionStore((s) => s.toggleTerminal);
  const setSettingsOpen = useSessionStore((s) => s.setSettingsOpen);
  const shareConversation = useSessionStore((s) => s.shareConversation);
  const signedIn = useSessionStore((s) => s.auth !== null);
  const title = useSessionStore((s) =>
    session ? s.conversations.find((c) => c.id === session.id)?.title : null,
  );
  const projectCwd = useSessionStore((s) => s.localSettings.cwd);
  const isLocal = session?.provider === "local";

  return (
    <header className="flex h-12 shrink-0 items-center gap-2.5 border-b border-border bg-bg-elevated px-4">
      {session && (
        <div className="flex min-w-0 items-center gap-2">
          <span
            className="size-1.5 shrink-0 rounded-full bg-success"
            title="Connected"
            aria-label="Connected"
          />
          {isLocal && projectCwd && (
            <span
              title={projectCwd}
              className="hidden shrink-0 items-center gap-1 rounded-md border border-border-subtle bg-bg-elevated px-1.5 py-0.5 text-xs font-medium text-ink-secondary md:flex"
            >
              <FolderGit2 className="size-3" />
              {projectName(projectCwd)}
            </span>
          )}
          <span className="truncate text-sm font-medium text-ink-secondary">
            {title ?? "New conversation"}
          </span>
        </div>
      )}

      <div className="ml-auto flex items-center gap-1">
        <UpdatePill />
        {session && signedIn && (
          <button
            onClick={() => void shareConversation()}
            aria-label="Share conversation"
            title="Copy a public read-only link (/unshare stops sharing)"
            className="grid size-8 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover hover:text-ink-secondary"
          >
            <Share2 className="size-4" />
          </button>
        )}
        {session && isLocal && <ChangesButton />}
        {session && isLocal && projectCwd && <MemoryButton />}
        <button
          onClick={() => setSettingsOpen(true)}
          aria-label="Settings"
          title="Settings (⌘,)"
          className="grid size-8 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover hover:text-ink-secondary"
        >
          <SettingsIcon className="size-4" />
        </button>
        {session && (
          <button
            onClick={toggleTerminal}
            aria-label={terminalOpen ? "Hide terminal" : "Show terminal"}
            title="Terminal (run commands in your project)"
            className={cn(
              "grid size-8 place-items-center rounded-lg transition",
              terminalOpen
                ? "bg-bg-hover text-ink"
                : "text-ink-muted hover:bg-bg-hover hover:text-ink-secondary",
            )}
          >
            <SquareTerminal className="size-4" />
          </button>
        )}
        <button
          onClick={onToggleTheme}
          aria-label={dark ? "Switch to light theme" : "Switch to dark theme"}
          className="grid size-8 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover"
        >
          {dark ? <Sun className="size-4" /> : <Moon className="size-4" />}
        </button>
        {auth && (
          <div className="ml-1">
            <ProfileMenu />
          </div>
        )}
      </div>
    </header>
  );
}
