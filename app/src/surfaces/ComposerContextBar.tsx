import { useEffect, useMemo, useState } from "react";
import { Folder, GitBranch, GitFork, Laptop, Server } from "lucide-react";
import type { ProjectContext, RemoteExecutorTarget } from "../core-bridge/bridge";
import { projectName } from "../lib/localAgent";
import { loadProjectContext } from "../lib/projectContext";
import { useSessionStore } from "../store/sessionStore";
import { BranchPicker } from "./BranchPicker";
import { EnvironmentPicker } from "./EnvironmentPicker";
import { ManagedWorktreeBasePicker } from "./ManagedWorktreeJourney";
import { ParallelWorkContext } from "./ParallelWorkContext";

const ITEM =
  "flex h-[22px] min-w-0 items-center gap-1 rounded-md bg-composer-context px-1.5 text-[11px] font-medium leading-none";

/** Checkout identity attached to the composer. Before a session starts the
 * project/environment fields remain interactive; once a session is live they
 * become read-only because that run is pinned to its original checkout. */
export function ComposerContextBar() {
  const session = useSessionStore((state) => state.session);
  const activeProvider = useSessionStore((state) => state.activeProvider);
  const projectMode = useSessionStore((state) => state.projectMode);
  const localCwd = useSessionStore((state) => state.localSettings.cwd);
  const setProjectFolder = useSessionStore((state) => state.setProjectFolder);
  const activeProjectRoot = useSessionStore((state) => state.activeProjectRoot);
  const activeRemote = useSessionStore((state) => state.activeRemote);
  const activeRemoteHost = useSessionStore((state) => state.activeRemoteHost);
  const conversations = useSessionStore((state) => state.conversations);
  const runningIds = useSessionStore((state) => state.runningIds);
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
  const [refreshTick, setRefreshTick] = useState(0);
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
  }, [canInspect, cwd, inspectionKey, remote, runState, refreshTick]);

  useEffect(() => {
    if (!canInspect) return;
    const timer = window.setInterval(() => setRefreshTick((tick) => tick + 1), 15_000);
    return () => window.clearInterval(timer);
  }, [canInspect, inspectionKey]);

  const isRemoteSession = Boolean(activeRemote);
  const locationLabel = activeRemoteHost?.trim() || (isRemoteSession ? "Remote" : "Local");
  const LocationIcon = isRemoteSession ? Server : Laptop;
  const checkoutRoot = context?.worktreeRoot || cwd;
  const CheckoutIcon = context?.isWorktree ? GitFork : GitBranch;
  const checkoutTone = context?.isWorktree
    ? "text-checkout-worktree"
    : "text-checkout-branch";
  const normalizedCheckout = checkoutRoot.replace(/\/+$/, "");
  const clarkPeers = conversations
    .filter((conversation) => {
      if (!runningIds.includes(conversation.id) || conversation.id === session?.id) return false;
      if ((conversation.project ?? "").replace(/\/+$/, "") !== normalizedCheckout) return false;
      return (conversation.remoteHost ?? null) === (activeRemoteHost ?? null);
    })
    .map((conversation) => ({ id: conversation.id, title: conversation.title }));
  const workingFiles = context
    ? context.activity.changedFiles
      + context.activity.untrackedFiles
      + context.activity.conflictedFiles
    : 0;
  const otherAgentCount = (context?.activity.externalAgents.length ?? 0) + clarkPeers.length;
  const branchSwitchDisabledReason = otherAgentCount > 0
    ? "Another agent is active in this checkout. Wait for it to finish before switching branches."
    : workingFiles > 0
      ? "Commit or remove local changes before switching branches."
      : undefined;
  const canSwitchBranch = !session && projectMode === "local" && !isRemoteSession;

  if (session && (activeProvider !== "local" || !checkoutRoot)) return null;

  return (
    // Keep this wrapper out of its own stacking layer. The context popovers
    // carry their own z-index; a parent z-index would paint these chips over
    // menus opened from the composer card below.
    <div className="composer-column-width relative mx-auto mb-1.5 w-full" data-testid="composer-context-bar">
      <div
        aria-label="Checkout context"
        data-readonly={session ? "true" : undefined}
        className="flex min-w-0 items-center justify-start gap-1.5"
      >
        {session ? (
          <>
            <span
              className={`${ITEM} shrink-0 text-ink-secondary`}
              title={isRemoteSession ? `Remote: ${locationLabel}` : "This Mac"}
            >
              <LocationIcon className="size-3 shrink-0" />
              <span className="max-w-36 truncate">{locationLabel}</span>
            </span>
            <span
              className={`${ITEM} text-ink-secondary`}
              title={`${context?.isWorktree ? "Linked worktree" : "Project"}: ${checkoutRoot}`}
            >
              <Folder className="size-3 shrink-0" />
              <span className="max-w-48 truncate">{projectName(checkoutRoot)}</span>
            </span>
          </>
        ) : (
          <EnvironmentPicker compact />
        )}

        {context && canSwitchBranch ? (
          <BranchPicker
            cwd={checkoutRoot}
            context={context}
            disabledReason={branchSwitchDisabledReason}
            allowPreserveChanges={workingFiles > 0 && otherAgentCount === 0}
            onSwitched={() => setRefreshTick((tick) => tick + 1)}
            onOpenCheckout={(path) => setProjectFolder(path)}
            onTransitionPlan={(plan) => {
              useSessionStore.setState({ worktreeTransition: plan, error: null });
            }}
          />
        ) : context ? (
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
            <CheckoutIcon className="size-3 shrink-0" />
            <span className="max-w-48 truncate">
              {context.detached ? `detached@${context.branch}` : context.branch}
            </span>
          </span>
        ) : null}
        {context && canSwitchBranch && <ManagedWorktreeBasePicker />}
        {context && (
          <ParallelWorkContext
            activity={context.activity}
            branch={context.detached ? `detached@${context.branch}` : context.branch}
            clarkPeers={clarkPeers}
          />
        )}
      </div>
    </div>
  );
}
