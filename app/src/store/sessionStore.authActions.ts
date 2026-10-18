import { type SessionState, type SessionSet, type SessionGet,
  codeKeyAccountBinding, configureCloudHistoryCredentials, cloudCreds,
  refreshAuthSession, resetCloudHistory, authSignOut, signInWithGoogle,
  snapshotCache, loadLocalSettings, loadChatModels, loadApprovalPolicies,
  loadMemoriesEnabled, loadBrowserEnabled, loadOrchestrationEnabled,
  loadApprovalPolicy, loadCollaborationMode, loadCollaborationModes,
  loadOutputStyle, loadSshHosts, loadRecentProjects,
} from "./sessionStore.runtime";
import { authAccountMatches } from "../lib/account";
import { setAuthConnection, cancelAuthSignIn } from "../lib/auth";
import { productAuthFailure } from "../product/productAuthFailure";
import { recoverProductAuthentication } from "../product/productBridge";
import { resetStableOrder } from "../lib/stableOrder";
import { loadManagedWorktreeBase } from "../lib/managedWorktreeSettings";
import { useSpecialistStore } from "./specialistStore";

/** Whether applying `next` would replace the account that owns local live
 * sessions and caches. The normal UI signs out before another account can sign
 * in, but keeping this boundary here makes the state safe for an auth handoff
 * or a programmatic re-auth too. A refreshed token for the same stable account
 * is deliberately not a switch. */
export function authAccountChanged(
  current: SessionState["auth"],
  next: NonNullable<SessionState["auth"]>,
): boolean {
  if (!current) return false;
  const currentOwner = codeKeyAccountBinding(current);
  const nextOwner = codeKeyAccountBinding(next);
  if (currentOwner && nextOwner) return currentOwner !== nextOwner;
  return true;
}

/** Rotate the native credential for the account that still owns the store.
 * Every authenticated product surface and native cloud event shares this
 * boundary through productBridge's single-flight recovery. */
export async function refreshActiveProductAuthentication(
  set: SessionSet,
  get: SessionGet,
): Promise<void> {
  const startedWith = get().auth;
  if (!startedWith) throw new Error("Sign in again to restore product access.");

  try {
    const refreshed = await refreshAuthSession(
      startedWith,
      () => authAccountMatches(startedWith, get().auth),
    );
    if (!authAccountMatches(startedWith, get().auth)) {
      throw new Error("The active account changed while Clark refreshed access.");
    }
    configureCloudHistoryCredentials(cloudCreds(refreshed));
    set({ auth: refreshed, authReconnectError: null });
  } catch (error) {
    const current = get().auth;
    if (current && authAccountMatches(startedWith, current)) {
      set({
        auth: setAuthConnection(current, productAuthFailure(error)?.code === "reconnect_required" ? "reconnect_required" : "offline"),
      });
    }
    throw error;
  }
}

function activateSignedInAccount(
  set: SessionSet,
  get: SessionGet,
  auth: NonNullable<SessionState["auth"]>,
): void {
  const replacingAccount = authAccountChanged(get().auth, auth);
  const accountScope = codeKeyAccountBinding(auth);
  resetCloudHistory();
  resetStableOrder();
  if (replacingAccount) {
    get().endSession({ force: true });
    snapshotCache.clear();
  }
  useSpecialistStore.getState().setAccountScope(accountScope);
  configureCloudHistoryCredentials(cloudCreds(auth));
  const accountLocalSettings = loadLocalSettings(accountScope);
  set({
    auth,
    localSettings: accountLocalSettings,
    managedWorktreeBase: loadManagedWorktreeBase(accountScope, accountLocalSettings.cwd),
    chatModels: loadChatModels(accountScope),
    approvalPolicies: loadApprovalPolicies(accountScope),
    memoriesEnabled: loadMemoriesEnabled(accountScope),
    browserEnabled: loadBrowserEnabled(accountScope),
    orchestrationEnabled: loadOrchestrationEnabled(accountScope),
    approvalPolicy: loadApprovalPolicy(accountScope),
    collaborationMode: loadCollaborationMode(accountScope),
    collaborationModes: loadCollaborationModes(accountScope),
    outputStyle: loadOutputStyle(accountScope),
    selectedHostId: loadSshHosts(accountScope)[0]?.id ?? null,
    projectMode: "local",
    recentProjects: loadRecentProjects(accountScope),
    memoryStatus: null,
    memoryViewerOpen: false,
    loadingMemory: false,
    memoryOverview: null,
    globalMemoryOverview: null,
    pendingManagedWorktreePath: null,
    terminalLaunch: null,
    mcpOpen: false,
    sshOpen: false,
    newProjectOpen: false,
    paletteOpen: false,
    error: null,
    notice: null,
    warning: null,
    dismissedFailedRuns: [],
    conversations: [],
    conversationsLoading: true,
    runningIds: [],
    unseenWorkIds: [],
    selectedConversationIds: new Set(),
    mutatingConversationIds: new Set(),
    conversationMutation: null,
  });
  void get().ensureCodeKey();
  void get().syncCloudIndex();
}

export function createAuthActions(set: SessionSet, get: SessionGet): Pick<SessionState, "signIn" | "reconnectAuth" | "cancelAuthReconnect" | "signOutAuth"> {
  let reconnectFlight: Promise<void> | null = null;
  let reconnectCancelled = false;
  return {
  signIn: async () => {
    const auth = await signInWithGoogle();
    activateSignedInAccount(set, get, auth);
  },

  reconnectAuth: () => {
    if (reconnectFlight) return reconnectFlight;
    const current = get().auth;
    if (!current) return Promise.resolve();
    reconnectCancelled = false;
    set({ authReconnecting: true, authReconnectError: null });
    let flight: Promise<void>;
    flight = Promise.resolve().then(async () => {
      try {
        try {
          await recoverProductAuthentication();
        } catch (error) {
          if (!authAccountMatches(current, get().auth)) return;
          if (reconnectCancelled) throw { code: "cancelled", message: "Sign-in cancelled." };
          if (productAuthFailure(error)?.code !== "reconnect_required") throw error;
          const reauthenticated = await signInWithGoogle();
          if (!authAccountMatches(current, get().auth)) return;
          if (!authAccountMatches(reauthenticated, current)) {
            throw {code:"account_changed", message:"Choose the same account to reconnect. Sign out before switching accounts."};
          }
          configureCloudHistoryCredentials(cloudCreds(reauthenticated));
          set({ auth: reauthenticated });
        }
        if (!authAccountMatches(current, get().auth)) return;
        set({ error: null, warning: null });
        await get().syncCloudIndex();
      } catch (error) {
        if (!authAccountMatches(current, get().auth)) return;
        const failure = productAuthFailure(error);
        set({ authReconnectError: failure?.code === "cancelled" ? "Sign-in cancelled. You can retry anytime." : failure?.message ?? "Could not reconnect. Please try again." });
      } finally {
        if (reconnectFlight === flight) {
          reconnectFlight = null;
          set({ authReconnecting: false });
        }
      }
    });
    reconnectFlight = flight;
    return flight;
  },

  cancelAuthReconnect: async () => {
    reconnectCancelled = true;
    try { await cancelAuthSignIn(); }
    catch { set({ authReconnectError: "Could not cancel sign-in. It will time out shortly." }); }
  },

  signOutAuth: async () => {
    // A Clark Code key is account-scoped. Removing the local binding prevents
    // the next user from silently consuming access owned by this account.
    resetCloudHistory();
    resetStableOrder();
    try {
      await authSignOut();
    } catch (error) {
      set({ error: `Could not sign out safely: ${String(error)}` });
      return;
    }
    get().endSession({ force: true });
    useSpecialistStore.getState().setAccountScope(null);
    // Drop the in-memory history entirely so the signed-out (and any next)
    // account starts clean.
    snapshotCache.clear();
    const anonymousLocalSettings = loadLocalSettings(null);
    set({
      auth: null,
      authReconnecting: false,
      authReconnectError: null,
      localSettings: anonymousLocalSettings,
      managedWorktreeBase: loadManagedWorktreeBase(null, anonymousLocalSettings.cwd),
      chatModels: loadChatModels(null),
      approvalPolicies: loadApprovalPolicies(null),
      memoriesEnabled: loadMemoriesEnabled(null),
      browserEnabled: loadBrowserEnabled(null),
      orchestrationEnabled: loadOrchestrationEnabled(null),
      approvalPolicy: loadApprovalPolicy(null),
      collaborationMode: loadCollaborationMode(null),
      collaborationModes: loadCollaborationModes(null),
      outputStyle: loadOutputStyle(null),
      selectedHostId: loadSshHosts(null)[0]?.id ?? null,
      projectMode: "local",
      recentProjects: [],
      memoryStatus: null,
      memoryViewerOpen: false,
      loadingMemory: false,
      memoryOverview: null,
      globalMemoryOverview: null,
      pendingManagedWorktreePath: null,
      terminalLaunch: null,
      mcpOpen: false,
      sshOpen: false,
      sshOpenPurpose: "manage",
      newProjectOpen: false,
      paletteOpen: false,
      error: null,
      notice: null,
      warning: null,
      dismissedFailedRuns: [],
      conversations: [],
      conversationsLoading: false,
      unseenWorkIds: [],
      selectedConversationIds: new Set(),
      mutatingConversationIds: new Set(),
      conversationMutation: null,
    });
  },

  };
}
