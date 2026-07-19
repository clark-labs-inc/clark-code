import { useEffect, useMemo, useState } from "react";
import { Folder, GitBranch, GitFork, Laptop, Server } from "lucide-react";
import type { ProjectContext, RemoteExecutorTarget } from "../core-bridge/bridge";
import { projectName } from "../lib/localAgent";
import { loadProjectContext } from "../lib/projectContext";
import { useSessionStore } from "../store/sessionStore";
import { EnvironmentPicker } from "./EnvironmentPicker";

const ITEM =
  "flex min-h-7 min-w-0 items-center gap-1.5 text-xs font-medium";

/** Checkout identity attached to the composer. Before a session starts the
 * project/environment fields remain interactive; once a session is live they
 * become read-only because that run is pinned to its original checkout. */
export function ComposerContextBar() {
  const session = useSessionStore((state) => state.session);
  const activeProvider = useSessionStore((state) => state.activeProvider);
  const projectMode = useSessionStore((state) => state.projectMode);
  const localCwd = useSessionStore((state) => state.localSettings.cwd);
  const activeProjectRoot = useSessionStore((state) => state.activeProjectRoot);
  const activeRemote = useSessionStore((state) => state.activeRemote);
  const activeRemoteHost = useSessionStore((state) => state.activeRemoteHost);
  const runState = useSessionStore((state) =>
    Object.values(state.snapshot.runs)
      .map((run) => `${run.id}:${run.status}`)
      .join("|"),
  );
  const cwd = session
    ? activeProjectRoot?.trim() || localCwd.trim()
    : localCwd.trim();
  const remote = useMemo<RemoteExecutorTarget | null>(
    () =>
      session && activeRemote
        ? { ws_url: activeRemote.ws_url, token: activeRemote.token }
        : null,
    [activeRemote, session],
  );
  const canInspect =
    activeProvider === "local" &&
    Boolean(cwd) &&
    (Boolean(session) || projectMode === "local");
  const inspectionKey = `${cwd}\u0000${remote?.ws_url ?? "local"}`;
  const [loadedContext, setLoadedContext] = useState<{
    key: string;
    value: ProjectContext | null;
  } | null>(null);
  const context =
    loadedContext?.key === inspectionKey ? loadedContext.value : null;

  useEffect(() => {
    let current = true;
    if (!canInspect) {
      setLoadedContext(null);
      return () => {
        current = false;
      };
    }

    void loadProjectContext(cwd, remote).then((next) => {
      if (current) setLoadedContext({ key: inspectionKey, value: next });
    });
    return () => {
      current = false;
    };
  }, [canInspect, cwd, inspectionKey, remote, runState]);

  const isRemoteSession = Boolean(activeRemote);
  const locationLabel = activeRemoteHost?.trim() || (isRemoteSession ? "Remote" : "Local");
  const LocationIcon = isRemoteSession ? Server : Laptop;
  const checkoutRoot = context?.worktreeRoot || cwd;
  const CheckoutIcon = context?.isWorktree ? GitFork : GitBranch;
  const checkoutTone = context?.isWorktree
    ? "text-checkout-worktree"
    : "text-checkout-branch";

  if (session && (activeProvider !== "local" || !checkoutRoot)) return null;

  return (
    <div className="relative mx-auto -mb-3 max-w-2xl px-3" data-testid="composer-context-bar">
      <div
        aria-label="Checkout context"
        data-readonly={session ? "true" : undefined}
        className="flex h-12 min-w-0 items-start justify-center gap-5 rounded-t-[20px] bg-composer-context px-[15px] pb-3.5 pt-1.5"
      >
        {session ? (
          <>
            <span
              className={`${ITEM} text-ink-secondary`}
              title={`${context?.isWorktree ? "Linked worktree" : "Project"}: ${checkoutRoot}`}
            >
              <Folder className="size-3.5 shrink-0" />
              <span className="max-w-48 truncate">{projectName(checkoutRoot)}</span>
            </span>
            <span
              className={`${ITEM} text-ink-secondary`}
              title={isRemoteSession ? `Remote: ${locationLabel}` : "This Mac"}
            >
              <LocationIcon className="size-3.5 shrink-0" />
              <span className="max-w-36 truncate">{locationLabel}</span>
            </span>
          </>
        ) : (
          <EnvironmentPicker compact />
        )}

        {context && (
          <span
            className={`${ITEM} shrink-0 ${checkoutTone}`}
            title={
              context.detached
                ? `Detached HEAD at ${context.branch}`
                : context.isWorktree
                  ? `Linked worktree on ${context.branch}`
                  : `Branch: ${context.branch}`
            }
          >
            <CheckoutIcon className="size-3.5 shrink-0" />
            <span className="max-w-48 truncate">
              {context.detached ? `detached@${context.branch}` : context.branch}
            </span>
          </span>
        )}
      </div>
    </div>
  );
}
