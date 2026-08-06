import { useEffect, useMemo, useState } from "react";
import { AlertTriangle, ChevronDown, Folder, GitBranch, GitFork, Laptop, Server } from "lucide-react";
import type { ProjectContext, RemoteWorkerTarget } from "../core-bridge/bridge";
import { projectDisplayName } from "../lib/projectSidebar";
import { loadProjectContext } from "../lib/projectContext";
import { codeKeyAccountBinding } from "../lib/account";
import { openRemote } from "../store/sessionStore.runtime";
import type { RemoteInfo } from "../lib/remoteWorker";
import { loadSshHosts, saveSshHosts } from "../lib/sshHosts";
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
  const selectedHostId = useSessionStore((state) => state.selectedHostId);
  const auth = useSessionStore((state) => state.auth);
  const localCwd = useSessionStore((state) => state.localSettings.cwd);
  const localSettings = useSessionStore((state) => state.localSettings);
  const setProjectFolder = useSessionStore((state) => state.setProjectFolder);
  const activeProjectRoot = useSessionStore((state) => state.activeProjectRoot);
  const activeRemote = useSessionStore((state) => state.activeRemote);
  const activeRemoteHost = useSessionStore((state) => state.activeRemoteHost);
  const conversations = useSessionStore((state) => state.conversations);
  const runningIds = useSessionStore((state) => state.runningIds);
  const openConversation = useSessionStore((state) => state.openConversation);
  const runState = useSessionStore((state) =>
    Object.values(state.snapshot.runs)
      .map((run) => `${run.id}:${run.status}`)
      .join("|"),
  );
  const accountScope = codeKeyAccountBinding(auth);
  const selectedHost = loadSshHosts(accountScope).find((host) => host.id === selectedHostId) ?? null;
  const isRemoteSelection = !session && activeProvider === "local" && projectMode === "remote";
  const cwd = session
    ? activeProjectRoot?.trim() || localCwd.trim()
    : isRemoteSelection
      ? selectedHost?.remoteRoot.trim() ?? ""
      : localCwd.trim();
  const [inspectionRemote, setInspectionRemote] = useState<RemoteInfo | null>(null);
  const [inspectionError, setInspectionError] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    setInspectionRemote(null);
    setInspectionError(null);
    if (!isRemoteSelection || !selectedHost) return () => { current = false; };

    void openRemote(selectedHost, localSettings, cwd).then((next) => {
      if (current) setInspectionRemote(next);
    }).catch((cause) => {
      if (current) {
        setInspectionError(cause instanceof Error ? cause.message : String(cause));
      }
    });
    return () => {
      current = false;
    };
  }, [cwd, isRemoteSelection, localSettings.model, localSettings.reasoningEffort, selectedHost?.host, selectedHost?.id]);

  const remote = useMemo<RemoteWorkerTarget | null>(
    () =>
      session && activeRemote
        ? { id: activeRemote.id }
        : inspectionRemote
          ? { id: inspectionRemote.id }
          : null,
    [activeRemote, inspectionRemote, session],
  );
  const canInspect =
    activeProvider === "local" &&
    Boolean(cwd) &&
    (Boolean(session) || projectMode === "local" || Boolean(remote));
  const inspectionKey = `${cwd}\u0000${remote?.id ?? "local"}`;
  const [loadedContext, setLoadedContext] = useState<{
    key: string;
    value: ProjectContext | null;
  } | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);
  const [mobileExpanded, setMobileExpanded] = useState(false);
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
  const isRemoteContext = isRemoteSession || isRemoteSelection;
  const locationLabel = activeRemoteHost?.trim()
    || selectedHost?.host.trim()
    || (isRemoteContext ? "Remote" : "Local");
  const LocationIcon = isRemoteContext ? Server : Laptop;
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
  const workingTreeLabel = context && workingFiles > 0
    ? [
        context.activity.changedFiles > 0
          ? `${context.activity.changedFiles} changed file${context.activity.changedFiles === 1 ? "" : "s"}`
          : "",
        context.activity.untrackedFiles > 0
          ? `${context.activity.untrackedFiles} untracked file${context.activity.untrackedFiles === 1 ? "" : "s"}`
          : "",
        context.activity.conflictedFiles > 0
          ? `${context.activity.conflictedFiles} conflicted file${context.activity.conflictedFiles === 1 ? "" : "s"}`
          : "",
      ].filter(Boolean).join(", ")
    : null;
  const otherAgentCount = (context?.activity.externalAgents.length ?? 0) + clarkPeers.length;
  const branchSwitchDisabledReason = otherAgentCount > 0
    ? `${otherAgentCount} other agent${otherAgentCount === 1 ? " is" : "s are"} active in this checkout.`
    : workingFiles > 0
      ? "Commit or remove local changes before switching branches."
      : undefined;
  const canSwitchBranch = !session && (projectMode === "local" || Boolean(remote));

  if (session && (activeProvider !== "local" || !checkoutRoot)) return null;

  return (
    // Keep this wrapper out of its own stacking layer. The context popovers
    // carry their own z-index; a parent z-index would paint these chips over
    // menus opened from the composer card below.
    <div className="composer-column-width relative mx-auto mb-1.5 w-full" data-testid="composer-context-bar">
      <button
        type="button"
        aria-expanded={mobileExpanded}
        aria-controls="composer-checkout-context"
        onClick={() => setMobileExpanded((expanded) => !expanded)}
        className="flex h-7 max-w-full items-center gap-1.5 rounded-lg bg-composer-context px-2 text-[11px] font-medium text-ink-secondary sm:hidden"
      >
        <LocationIcon className="size-3 shrink-0" />
        <span className="truncate">
          Context · {locationLabel}{checkoutRoot ? ` · ${projectDisplayName(checkoutRoot)}` : ""}
        </span>
        <ChevronDown className={`size-3 shrink-0 transition ${mobileExpanded ? "rotate-180" : ""}`} />
      </button>
      <div
        id="composer-checkout-context"
        aria-label="Checkout context"
        data-readonly={session ? "true" : undefined}
        // Context menus are positioned above these chips. An overflow-x
        // scroller clips that vertical layer, leaving a branch/worktree menu
        // in the accessibility tree but unclickable in the real window. Let
        // the compact context row wrap instead; every action remains reachable
        // at narrow widths and the popovers keep their own z-index.
        className={`${mobileExpanded ? "flex" : "hidden"} mt-1.5 min-w-0 max-w-full flex-wrap items-center justify-start gap-1.5 overflow-visible sm:mt-0 sm:flex`}
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
              <span className="max-w-48 truncate">{projectDisplayName(checkoutRoot)}</span>
            </span>
          </>
        ) : (
          <EnvironmentPicker compact />
        )}

        {context && canSwitchBranch ? (
          <BranchPicker
            cwd={checkoutRoot}
            context={context}
            remote={remote}
            disabledReason={branchSwitchDisabledReason}
            allowPreserveChanges={workingFiles > 0 && otherAgentCount === 0}
            onSwitched={() => setRefreshTick((tick) => tick + 1)}
            onOpenCheckout={(path) => {
              if (!isRemoteSelection || !selectedHost) {
                setProjectFolder(path);
                return;
              }
              const hosts = loadSshHosts(accountScope);
              saveSshHosts(
                hosts.map((host) => host.id === selectedHost.id ? { ...host, remoteRoot: path } : host),
                accountScope,
              );
              useSessionStore.setState({ selectedHostId: selectedHost.id });
              setRefreshTick((tick) => tick + 1);
            }}
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
        {!session && workingTreeLabel && (
          <span
            className={`${ITEM} shrink-0 text-warning`}
            title={`Uncommitted work in this checkout: ${workingTreeLabel}`}
            aria-label={`Working tree has ${workingTreeLabel}`}
          >
            <AlertTriangle className="size-3 shrink-0" />
            <span className="max-w-40 truncate">Working tree · {workingTreeLabel}</span>
          </span>
        )}
        {!session && inspectionError && (
          <span
            className={`${ITEM} shrink-0 text-danger`}
            title={inspectionError}
            aria-label={`Remote Git inspection unavailable: ${inspectionError}`}
          >
            <AlertTriangle className="size-3 shrink-0" />
            <span className="max-w-48 truncate">Remote Git unavailable</span>
          </span>
        )}
        {context && canSwitchBranch && !remote && <ManagedWorktreeBasePicker />}
        {context && (
          <ParallelWorkContext
            activity={context.activity}
            branch={context.detached ? `detached@${context.branch}` : context.branch}
            clarkPeers={clarkPeers}
            onOpenPeer={(id) => {
              void openConversation(id);
            }}
          />
        )}
      </div>
    </div>
  );
}
