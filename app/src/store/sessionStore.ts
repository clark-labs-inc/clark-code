import { create } from "zustand";
import { createAppActions } from "./sessionStore.appActions";
import { createConversationActions } from "./sessionStore.conversationActions";
import { createInteractionActions } from "./sessionStore.interactionActions";
import {
      bootAuth,
  emptySnapshot,
  loadApprovalPolicy,
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

export {
  latestRunFailed,
  mergeHistory,
} from "./sessionStore.runtime";
export type {
  ComposerPrefill,
  QueuedMessage,
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
  selectedConversationIds: new Set<string>(),
  mutatingConversationIds: new Set<string>(),
  conversationMutation: null,
  opening: null,
  composerPrefill: null,
  localSettings: loadLocalSettings(),
  managedWorktreeBase: loadManagedWorktreeBase(),
  worktreeTransition: null,
  pendingManagedWorktreePath: null,
  worktreePreparing: false,
  chatModels: loadChatModels(),
  projectMode: "local",
  selectedHostId: loadSshHosts()[0]?.id ?? null,
  activeRemote: null,
  activeRemoteHost: null,
  activeProjectRoot: null,
  memoriesEnabled: loadMemoriesEnabled(),
  browserEnabled: loadBrowserEnabled(),
  orchestrationEnabled: loadOrchestrationEnabled(),
  memoryStatus: null,
  memoryViewerOpen: false,
  loadingMemory: false,
  memoryOverview: null,
  globalMemoryOverview: null,
  recentProjects: loadRecentProjects(),
  queued: [],
  approvalPolicy: loadApprovalPolicy(),
  collaborationMode: loadCollaborationMode(),
  outputStyle: loadOutputStyle(),
  terminalOpen: false,
  terminalLaunch: null,
  mcpOpen: false,
  sshOpen: false,
  settingsOpen: false,
  settingsSection: "general",
  paletteOpen: false,
  sideQuestion: null,
  sidebarCollapsed: false,
  billing: null,
  loadingBilling: false,
  activityReward: null,
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

// Dev-only test seam: lets headless harnesses inject store state (e.g. a low
// credit balance) to exercise UI that depends on the live backend. Stripped from
// production builds.
if (import.meta.env.DEV && typeof window !== "undefined") {
  (window as unknown as { __clarkStore?: typeof useSessionStore }).__clarkStore =
    useSessionStore;
}
