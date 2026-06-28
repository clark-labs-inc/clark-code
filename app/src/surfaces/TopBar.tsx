import { Sun, Moon, FolderGit2, SquareTerminal, Blocks } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
import { cn } from "../lib/cn";
import { MemoryButton } from "./MemoryPanel";
import { ProfileMenu } from "./ProfileMenu";

export function TopBar({ dark, onToggleTheme }: { dark: boolean; onToggleTheme: () => void }) {
  const session = useSessionStore((s) => s.session);
  const auth = useSessionStore((s) => s.auth);
  const terminalOpen = useSessionStore((s) => s.terminalOpen);
  const toggleTerminal = useSessionStore((s) => s.toggleTerminal);
  const setMcpOpen = useSessionStore((s) => s.setMcpOpen);
  const title = useSessionStore((s) =>
    session ? s.conversations.find((c) => c.id === session.id)?.title : null,
  );
  const projectCwd = useSessionStore((s) => s.localSettings.cwd);
  const isLocal = session?.provider === "local";

  return (
    <header className="flex h-12 shrink-0 items-center gap-2.5 border-b border-border bg-bg-elevated/70 px-4 backdrop-blur">
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
        {session && isLocal && projectCwd && <MemoryButton />}
        <button
          onClick={() => setMcpOpen(true)}
          aria-label="MCP servers"
          title="MCP servers — extend Clark Code with external tools"
          className="grid size-8 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover hover:text-ink-secondary"
        >
          <Blocks className="size-4" />
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
