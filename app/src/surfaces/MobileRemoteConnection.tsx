import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { useSessionStore } from "../store/sessionStore";
import { authConnection } from "../lib/auth";
import { cloudCreds } from "../lib/cloudHistory";
import { CODE_REMOTE_CAPABILITIES, CODE_REMOTE_PROTOCOL_VERSION, pollCodeRemoteCommands, registerCodeRemoteHost } from "../lib/mobileRemote";
import { discoverRepositories, projectKnowledgeEnabled, syncRepositoriesUnderRoot } from "../lib/repositoryKnowledge";
import { desktopInstanceId, desktopIdentity } from "../lib/desktopHost";
import { codeKeyAccountBinding } from "../lib/account";
import { mobileRemoteRetryDelayMs } from "../lib/mobileRemoteRetry";
import { MobileRemotePresenceLoop, publishMobileRemotePresence } from "../lib/mobileRemotePresence";
import { currentProjects, commandWaitsForTargetIdle, runCommand } from "./MobileRemoteAgent";
import { desktopRemoteModels } from "../lib/mobileRemoteModelSettings";
const LOOP_INTERVAL_MS = 500;
const COMMAND_POLL_WAIT_MS = 25_000;

export function MobileRemoteAgent() {
  const [registrationFailed, setRegistrationFailed] = useState(false);
  const [pollingFailed, setPollingFailed] = useState(false);
  const auth = useSessionStore((state) => state.auth);
  const cwd = useSessionStore((state) => state.localSettings.cwd);
  const owner = codeKeyAccountBinding(auth);
  const connection = authConnection(auth);

  useEffect(() => {
    setRegistrationFailed(false);
    setPollingFailed(false);
    if (!owner) return;
    const effectOwner = owner;
    const identity = desktopIdentity;
    const instanceId = desktopInstanceId();
    let stopped = false;
    let appVersion: Promise<string> | null = null;
    let commandBusy = false;
    let repositoryBusy = false;
    let consecutiveFailures = 0;
    let retryAt = 0;

    const accountStillCurrent = () => {
      const current = useSessionStore.getState().auth;
      const currentOwner = codeKeyAccountBinding(current);
      return Boolean(effectOwner && effectOwner === currentOwner);
    };

    const refreshPresence = async () => {
      if (stopped || !accountStillCurrent() || navigator.onLine === false) return;
      const creds = cloudCreds(useSessionStore.getState().auth);
      if (!creds) {
        setRegistrationFailed(true);
        return;
      }
      const root = useSessionStore.getState().localSettings.cwd.trim();
      const refreshRepositories = root
        && projectKnowledgeEnabled(creds.accountScope)
        && !repositoryBusy
        ? async () => {
            repositoryBusy = true;
            try {
              await discoverRepositories(root, creds.accountScope);
              if (stopped || !accountStillCurrent()) return;
              const currentCreds = cloudCreds(useSessionStore.getState().auth);
              if (currentCreds) await syncRepositoriesUnderRoot(currentCreds, root);
            } finally {
              repositoryBusy = false;
            }
          }
        : undefined;
      try {
        await publishMobileRemotePresence(
          async () => {
            appVersion ??= getVersion().catch((error) => {
              appVersion = null;
              throw error;
            });
            const version = await appVersion;
            const { id: hostId, name } = await identity();
            if (stopped || !accountStillCurrent()) return;
            const currentCreds = cloudCreds(useSessionStore.getState().auth);
            if (!currentCreds) return;
            await registerCodeRemoteHost(currentCreds, {
              hostId,
              displayName: name,
              models: desktopRemoteModels(),
              os: navigator.platform || "desktop",
              arch: "",
              appVersion: version,
              protocolVersion: CODE_REMOTE_PROTOCOL_VERSION,
              capabilities: CODE_REMOTE_CAPABILITIES,
              projects: currentProjects(),
            });
            if (!stopped && accountStillCurrent()) setRegistrationFailed(false);
          },
          refreshRepositories,
        );
      } catch {
        if (!stopped && accountStillCurrent()) setRegistrationFailed(true);
      }
    };

    const presenceLoop = new MobileRemotePresenceLoop(refreshPresence);

    const pollCommands = async () => {
      if (stopped || !accountStillCurrent() || commandBusy) return;
      if (navigator.onLine === false || Date.now() < retryAt) return;
      const creds = cloudCreds(useSessionStore.getState().auth);
      if (!creds) {
        setRegistrationFailed(true);
        return;
      }
      commandBusy = true;
      try {
        const { id: hostId } = await identity();
        const response = await pollCodeRemoteCommands(
          creds,
          hostId,
          instanceId,
          1,
          COMMAND_POLL_WAIT_MS,
        );
        if (stopped || !accountStillCurrent()) return;
        for (const command of response.commands) {
          if (stopped) break;
          // Leave a busy follow-up in `delivered`, where it survives a desktop
          // restart. Acknowledging it and handing it to `send()` would put it
          // in the process-local queue and falsely report completion to mobile.
          if (commandWaitsForTargetIdle(command)) continue;
          await runCommand(
            creds,
            hostId,
            instanceId,
            command,
            () => !stopped && (
              cloudCreds(useSessionStore.getState().auth)?.accountScope === creds.accountScope
            ),
          );
        }
        consecutiveFailures = 0;
        if (!stopped && accountStillCurrent()) setPollingFailed(false);
        retryAt = 0;
      } catch {
        consecutiveFailures += 1;
        if (!stopped && accountStillCurrent() && consecutiveFailures >= 3) setPollingFailed(true);
        retryAt = Date.now() + mobileRemoteRetryDelayMs(consecutiveFailures);
        /* Remote control is a background affordance; normal desktop use continues. */
      } finally {
        commandBusy = false;
      }
    };

    presenceLoop.start();
    void pollCommands();
    const timer = window.setInterval(() => void pollCommands(), LOOP_INTERVAL_MS);
    const resumeAfterOutage = () => {
      consecutiveFailures = 0;
      retryAt = 0;
      presenceLoop.refreshNow();
      void pollCommands();
    };
    const resumeWhenVisible = () => {
      if (document.visibilityState === "visible") resumeAfterOutage();
    };
    window.addEventListener("online", resumeAfterOutage);
    window.addEventListener("focus", resumeAfterOutage);
    document.addEventListener("visibilitychange", resumeWhenVisible);
    return () => {
      stopped = true;
      presenceLoop.stop();
      window.clearInterval(timer);
      window.removeEventListener("online", resumeAfterOutage);
      window.removeEventListener("focus", resumeAfterOutage);
      document.removeEventListener("visibilitychange", resumeWhenVisible);
    };
  }, [owner, connection, cwd]);

  if (!auth || connection !== "ready" || (!registrationFailed && !pollingFailed)) return null;
  return <div role="status" className="border-b border-border bg-surface px-4 py-2 text-sm text-ink">
    Phone access to this computer is unavailable. Clark Code is retrying.
    <button className="ml-3 underline" onClick={() => useSessionStore.getState().setSettingsOpen(true)}>Settings</button>
  </div>;
}
