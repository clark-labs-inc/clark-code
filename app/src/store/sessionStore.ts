import { create } from "zustand";
import { createAppActions } from "./sessionStore.appActions";
import { createConversationActions } from "./sessionStore.conversationActions";
import { createInteractionActions } from "./sessionStore.interactionActions";
import {
      bootAuth,
  codeKeyAccountBinding,
  emptySnapshot,
  loadApprovalPolicy,
  loadApprovalPolicies,
  loadBrowserEnabled,
  loadChatModels,
  loadCollaborationMode,
  loadLocalSettings,
  loadMemoriesEnabled,
  loadOrchestrationEnabled,
  loadOutputStyle,
  loadRecentProjects,
  loadSshHosts,
} from "./sessionStore.runtime";
import { loadManagedWorktreeBase } from "../lib/managedWorktreeSettings";
import { useSpecialistStore } from "./specialistStore";

const bootAccountScope = codeKeyAccountBinding(bootAuth);
const bootLocalSettings = loadLocalSettings(bootAccountScope);

export {
  latestRunFailed,
  mergeHistory,
} from "./sessionStore.runtime";
export type {
  ComposerPrefill,
  QueuedMessage,
  SendOutcome,
  SkillReferenceBlock,
  SessionState,
  SettingsSection,
  SideQuestionState,
} from "./sessionStore.runtime";

export const useSessionStore = create<import("./sessionStore.runtime").SessionState>((set, get) => ({
  bridge: null,
  providers: [],
  activeProvider: null,
  session: null,
  snapshot: emptySnapshot(),
  connecting: false,
  error: null,
  notice: null,
  warning: null,
  dismissedFailedRuns: [],
  auth: bootAuth,
  attachments: [],
  conversations: [],
  conversationsLoading: !!bootAuth,
  historyPrefix: null,
  runningIds: [],
  unseenWorkIds: [],
  selectedConversationIds: new Set<string>(),
  mutatingConversationIds: new Set<string>(),
  conversationMutation: null,
  opening: null,
  unavailableConversation: null,
  unavailableCleanupId: null,
  composerPrefill: null,
  localSettings: bootLocalSettings,
  managedWorktreeBase: loadManagedWorktreeBase(bootAccountScope, bootLocalSettings.cwd),
  worktreeTransition: null,
  dirtyWorktreeApproval: null,
  pendingManagedWorktreePath: null,
  deferredSessionStartDraft: null,
  worktreePreparing: false,
  chatModels: loadChatModels(bootAccountScope),
  approvalPolicies: loadApprovalPolicies(bootAccountScope),
  projectMode: "local",
  selectedHostId: loadSshHosts(bootAccountScope)[0]?.id ?? null,
  activeRemote: null,
  activeRemoteHost: null,
  activeProjectRoot: null,
  memoriesEnabled: loadMemoriesEnabled(bootAccountScope),
  browserEnabled: loadBrowserEnabled(bootAccountScope),
  orchestrationEnabled: loadOrchestrationEnabled(bootAccountScope),
  memoryStatus: null,
  memoryViewerOpen: false,
  loadingMemory: false,
  memoryOverview: null,
  globalMemoryOverview: null,
  recentProjects: loadRecentProjects(bootAccountScope),
  queued: [],
  approvalPolicy: loadApprovalPolicy(bootAccountScope),
  collaborationMode: loadCollaborationMode(bootAccountScope),
  outputStyle: loadOutputStyle(bootAccountScope),
  terminalOpen: false,
  terminalLaunch: null,
  mcpOpen: false,
  sshOpen: false,
  newProjectOpen: false,
  settingsOpen: false,
  settingsSection: "general",
  paletteOpen: false,
  sideQuestion: null,
  sidebarCollapsed: false,
  update: null,
  updateProgress: null,
  updateChecking: false,
  updateApplying: false,
  updateWaiting: false,
  justUpdatedTo: null,


  ...createAppActions(set, get),
  ...createConversationActions(set, get),
  ...createInteractionActions(set, get),
}));

// A checkout starting point belongs to the specialist that will own the next
// chat. Switching specialists restores that specialist's account + project
// scoped choice synchronously, while ordinary Code chats retain their own
// existing preference under the unqualified key.
useSpecialistStore.subscribe((state, previous) => {
  if (state.active === previous.active) return;
  const sessionState = useSessionStore.getState();
  useSessionStore.setState({
    managedWorktreeBase: loadManagedWorktreeBase(
      codeKeyAccountBinding(sessionState.auth),
      sessionState.localSettings.cwd,
      state.active,
    ),
  });
});

// Dev-only test seam: lets headless harnesses inject store state (e.g. a low
// credit balance) to exercise UI that depends on the live backend. Stripped from
// production builds.
if (import.meta.env.DEV && typeof window !== "undefined") {
  (window as unknown as { __agentDesktopStore?: typeof useSessionStore }).__agentDesktopStore =
    useSessionStore;
}
