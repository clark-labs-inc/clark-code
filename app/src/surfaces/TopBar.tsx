import { Sun, Moon, FolderGit2, SquareTerminal, Settings as SettingsIcon, RefreshCw } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
import { cn } from "../lib/cn";
import { MemoryButton } from "./MemoryPanel";
import { ProfileMenu } from "./ProfileMenu";

/** Non-blocking "an update is downloaded — restart to apply" affordance. Shows
 *  only when a newer version has been staged; clicking relaunches into it. */
function UpdatePill() {
  const update = useSessionStore((s) => s.update);
  const apply = useSessionStore((s) => s.applyUpdate);
  if (!update) return null;
  return (
    <button
      onClick={() => void apply()}
      title={`Clark Code ${update.version} is ready — relaunch to update`}
      className="flex items-center gap-1.5 rounded-lg bg-accent/15 px-2.5 py-1 text-xs font-medium text-accent transition hover:bg-accent/25"
    >
      <RefreshCw className="size-3.5" />
      Restart to update
    </button>
  );
}

export function TopBar({ dark, onToggleTheme }: { dark: boolean; onToggleTheme: () => void }) {
  const session = useSessionStore((s) => s.session);
  const auth = useSessionStore((s) => s.auth);
  const terminalOpen = useSessionStore((s) => s.terminalOpen);
  const toggleTerminal = useSessionStore((s) => s.toggleTerminal);
  const setSettingsOpen = useSessionStore((s) => s.setSettingsOpen);
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
