import { Sun, Moon } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";

export function TopBar({ dark, onToggleTheme }: { dark: boolean; onToggleTheme: () => void }) {
  const session = useSessionStore((s) => s.session);
  const auth = useSessionStore((s) => s.auth);
  const signOutAuth = useSessionStore((s) => s.signOutAuth);
  const title = useSessionStore((s) =>
    session ? s.conversations.find((c) => c.id === session.id)?.title : null,
  );

  return (
    <header className="flex h-12 shrink-0 items-center gap-2.5 border-b border-border bg-bg-elevated/70 px-4 backdrop-blur">
      {session && (
        <div className="flex min-w-0 items-center gap-2">
          <span
            className="size-1.5 shrink-0 rounded-full bg-success"
            title="Connected"
            aria-label="Connected"
          />
          <span className="truncate text-sm font-medium text-ink-secondary">
            {title ?? "New conversation"}
          </span>
        </div>
      )}

      <div className="ml-auto flex items-center gap-1">
        <button
          onClick={onToggleTheme}
          aria-label={dark ? "Switch to light theme" : "Switch to dark theme"}
          className="grid size-8 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover"
        >
          {dark ? <Sun className="size-4" /> : <Moon className="size-4" />}
        </button>
        {auth && (
          <button
            onClick={signOutAuth}
            title={`Sign out${auth.user.email ? " (" + auth.user.email + ")" : ""}`}
            aria-label="Sign out"
            className="ml-1 flex items-center gap-1.5"
          >
            {auth.user.avatar ? (
              <img src={auth.user.avatar} alt="" className="size-7 rounded-full" />
            ) : (
              <span className="grid size-7 place-items-center rounded-full bg-bg-tertiary text-xs font-semibold text-ink-secondary transition hover:bg-bg-hover">
                {auth.user.name.charAt(0).toUpperCase()}
              </span>
            )}
          </button>
        )}
      </div>
    </header>
  );
}
