import type { StoreApi } from "zustand";
import type { SidebarConversationMutation } from "../lib/sidebarConversationInteractions";
import type {
  AuthMethod, SkillReferenceBlock, SendOutcome, ApprovalPolicy, AuthSession, ChatModelOverride, CollaborationMode, ConversationMeta, CoreBridge, DownloadProgress, LocalAgentSettings, ManagedWorktreeBase, MemoryOverview, PendingAttachment, PlanImplementationContext, ProjectWorktreeTransitionPlan, ProviderInfo, RemoteInfo, Session, Snapshot, StagedUpdate, UpdateCheckResult, QueuedMessage, ComposerPrefill, UnavailableConversation, SideQuestionState, SshOpenPurpose, SettingsSection, NewProjectTarget
} from "./sessionStore.runtime";

export interface SessionState {
  bridge: CoreBridge | null;
  providers: ProviderInfo[];
  activeProvider: string | null;
  session: Session | null;
  snapshot: Snapshot;
  connecting: boolean;
  error: string | null;
  /** Transient success/info toast (e.g. "Share link copied"). Auto-dismisses. */
  notice: string | null;
  /** Transient non-fatal warning toast (e.g. cloud sync hiccup mid-run). */
  warning: string | null;
  /** Run ids whose failed/stopped terminal banner was dismissed this session. */
  dismissedFailedRuns: string[];
  /** Authenticated user + the agent connection config it carries. */
  auth: AuthSession | null;
  authReconnecting: boolean;
  authReconnectError: string | null;
  /** Files staged to send with the next message. */
  attachments: PendingAttachment[];
  /** Saved conversations, newest first — the cloud is the source of truth. */
  conversations: ConversationMeta[];
  /** True while the first cloud conversation-list fetch is in flight. */
  conversationsLoading: boolean;
  /** Restored transcript when a past conversation is reopened (prefix to live). */
  historyPrefix: Snapshot | null;
  /** Conversation ids whose live session currently has a running run — drives
   *  the per-row "Working…" indicator in the sidebar. Any number of sessions
   *  can stream at once; switching between them never cancels a run. */
  runningIds: string[];
  /** Conversation ids whose background run finished while the user was in
   *  another conversation — a blue "finished, not yet visited" marker in the
   *  sidebar until the user opens it. In-memory only; cleared on visit or when
   *  the conversation is archived/deleted. */
  unseenWorkIds: string[];
  /** Conversation ids selected in the sidebar (Shift-click). Drives the
   *  right-click bulk actions (archive / delete all selected). A fresh Set
   *  on every mutation so zustand re-renders. */
  selectedConversationIds: Set<string>;
  /** Conversations currently awaiting a durable archive, delete, or restore
   * acknowledgement. Rows stay visible and explicitly busy until it arrives. */
  mutatingConversationIds: Set<string>;
  /** Sidebar bulk/single-item operation progress, kept briefly after completion
   * so visual and screen-reader feedback never disappears abruptly. */
  conversationMutation: SidebarConversationMutation | null;
  /** A session attach is in flight — drives the sidebar row spinner and, when
   *  no cached transcript exists, the opening screen. `kind` picks the copy
   *  ("Connecting" for a new session, "Reconnecting"/"Opening" for a reopen).
   *  Cleared when the session is live, the connect fails, or the user cancels
   *  (endSession). */
  opening: {
    id: string | null;
    kind: "start" | "open";
    title: string;
    remoteHost: string | null;
  } | null;
  /** The selected conversation when reopening failed. The workspace renders a
   * dedicated recovery surface while the matching sidebar row stays active. */
  unavailableConversation: UnavailableConversation | null;
  /** The unavailable entry being removed from its dedicated recovery surface.
   * Keeps that surface mounted until the full cleanup reset is ready to commit. */
  unavailableCleanupId: string | null;
  /** Text staged into the composer, optionally tied to a sent turn to replace. */
  composerPrefill: ComposerPrefill | null;
  /** Config for the "Local coding" provider (persisted to localStorage). */
  localSettings: LocalAgentSettings;
  /** Preferred immutable base for newly created managed worktrees. */
  managedWorktreeBase: ManagedWorktreeBase;
  /** A clean, isolated worktree awaiting the user's deliberate branch choice. */
  worktreeTransition: ProjectWorktreeTransitionPlan | null;
  /** A managed checkout already created for a start that can be retried. */
  pendingManagedWorktreePath: string | null;
  /** True while the host prepares a managed checkout for the next session. */
  worktreePreparing: boolean;
  /** Per-conversation model + reasoning-effort settings, keyed by conversation
   *  id and pinned when the chat is created or first reopened. Legacy chats
   *  fall back to `localSettings` only until that first open. Persisted to
   *  localStorage (the cloud stores transcripts, not model prefs). */
  chatModels: Record<string, ChatModelOverride>;
  /** Per-conversation approval-policy overrides, keyed by conversation id. A
   *  chat with no entry falls back to the account's global `approvalPolicy`
   *  default. Mirrors `chatModels`: each chat keeps its own level so cycling
   *  approval in one conversation never edits what another runs. Persisted to
   *  localStorage (the cloud stores transcripts, not prefs). */
  approvalPolicies: Record<string, ApprovalPolicy>;
  /** Where the next session runs: this machine, or a remote host over SSH. */
  projectMode: "local" | "remote";
  /** The saved SSH host selected for a remote session (id into sshHosts). */
  selectedHostId: string | null;
  /** Bumped whenever the persisted SSH host list changes (remote folder picked,
   *  host saved/edited) so UI reading `startBlockedReason()` — which re-reads
   *  localStorage — re-evaluates its blocked state. */
  sshHostsRevision: number;
  /** The native worker attachment for the active session (null when local). */
  activeRemote: RemoteInfo | null;
  /** The SSH destination of the active remote session, for the history badge. */
  activeRemoteHost: string | null;
  /** Authoritative project root for the active conversation. Unlike
   * `localSettings.cwd`, this does not change when the new-session picker does. */
  activeProjectRoot: string | null;
  /** Whether durable memory is enabled (global user preference; the agent gets
   *  the `memory` tool and its saved facts are injected into the prompt). */
  memoriesEnabled: boolean;
  /** Whether the experimental `browser` tool is enabled (off by default —
   *  downloads managed browser, ~150-300MB, on first use). */
  browserEnabled: boolean;
  /** Whether bounded parallel repository work is available. The model-facing
   *  policy still requires an explicit trigger; writers run in safe copies. */
  orchestrationEnabled: boolean;
  /** Last memory status message (e.g. a load error). */
  memoryStatus: string | null;
  /** Whether the memory viewer popover is open. */
  memoryViewerOpen: boolean;
  /** True while the memory viewer is (re)loading. */
  loadingMemory: boolean;
  /** The last-loaded project-scope memory for the active folder. */
  memoryOverview: MemoryOverview | null;
  /** The last-loaded global-scope (per-user) memory. */
  globalMemoryOverview: MemoryOverview | null;
  /** Recently opened project folders (newest first). */
  recentProjects: string[];
  /** Follow-up messages sent while a run is active; drained when it finishes. */
  queued: QueuedMessage[];
  /** How agent permission requests are approved. */
  approvalPolicy: ApprovalPolicy;
  /** Read-only planning is independent from action approval policy. */
  collaborationMode: CollaborationMode;
  /** Per-conversation collaboration-mode overrides, keyed by conversation id.
   *  A chat with no entry falls back to the account's global `collaborationMode`
   *  default. Mirrors `approvalPolicies`: each chat keeps its own mode so
   *  switching plan mode in one conversation never edits what another runs.
   *  Persisted to localStorage (the cloud stores transcripts, not prefs). */
  collaborationModes: Record<string, CollaborationMode>;
  /** The agent's reply tone/persona for this session — see `lib/outputStyle.ts`. */
  outputStyle: string;
  /** Whether the in-chat terminal drawer is open. */
  terminalOpen: boolean;
  /** One-shot request to open a fresh terminal tab rooted at `cwd`. `nonce`
   *  increments per request so the panel only reacts to the latest one. */
  terminalLaunch: { cwd: string; nonce: number } | null;
  /** Whether the MCP servers settings modal is open. */
  mcpOpen: boolean;
  /** Whether the remote-hosts (SSH) settings modal is open. */
  sshOpen: boolean;
  /** Why the SSH modal was opened. Adding a host from the execution picker
   *  must return that host to the picker; Settings only manages presets. */
  sshOpenPurpose: SshOpenPurpose;
  /** Whether the "New project" chooser modal is open. */
  newProjectOpen: boolean;
  /** Whether the unified Settings modal is open, and which section it shows. */
  settingsOpen: boolean;
  settingsSection: SettingsSection;
  /** Whether the ⌘K command palette is open. */
  paletteOpen: boolean;
  /** `/btw` side-question overlay state. A forked, tool-less model call over
   *  the session context that never interrupts the active run. Null when the
   *  overlay is closed. */
  sideQuestion: SideQuestionState | null;
  /** Whether the sidebar is collapsed to its icon rail. */
  sidebarCollapsed: boolean;
  /** A downloaded + staged app update awaiting a relaunch to apply. */
  update: StagedUpdate | null;
  /** Live byte progress while an update downloads in the background; null when idle. */
  updateProgress: DownloadProgress | null;
  /** A manifest check is in flight, including the gap before download progress starts. */
  updateChecking: boolean;
  /** True from "Restart to update" being clicked until the relaunch takes. */
  updateApplying: boolean;
  /** An update was requested and is waiting for active/queued work to settle. */
  updateWaiting: boolean;
  /** Set once on the first launch after an update applied — the version we're now on. */
  justUpdatedTo: string | null;

  init: () => Promise<void>;
  selectProvider: (id: string) => void;
  setLocalSettings: (patch: Partial<LocalAgentSettings>) => void;
  setManagedWorktreeBase: (base: ManagedWorktreeBase) => void;
  setProjectMode: (mode: "local" | "remote") => void;
  setSelectedHostId: (id: string | null) => void;
  /** Signal that the persisted SSH host list changed (a remote folder was set
   *  or a host was saved), so `startBlockedReason` re-evaluates. */
  bumpSshHostsRevision: () => void;
  setProjectFolder: (path: string) => void;
  /** Open the native folder picker and return the selected path after it has
   *  been committed to account-scoped project state. */
  pickProjectFolder: () => Promise<string | null>;
  setMemoriesEnabled: (on: boolean) => void;
  setBrowserEnabled: (on: boolean) => void;
  setOrchestrationEnabled: (on: boolean) => void;
  loadMemory: () => Promise<void>;
  toggleMemoryViewer: () => void;
  setMemoryViewerOpen: (open: boolean) => void;
  signIn: (method: AuthMethod) => Promise<void>;
  reconnectAuth: () => Promise<void>;
  cancelAuthReconnect: () => Promise<void>;
  signOutAuth: () => Promise<void>;
  /** Mint + store an Clark Code API key for the signed-in user if none yet. */
  ensureCodeKey: () => Promise<void>;
  /** Fetch the account's conversation list from the cloud (the source of truth). */
  syncCloudIndex: () => Promise<void>;
  /** Why the active environment can't start a session yet (folder unset, remote
   *  host not ready…), or null when ready. Lets the composer gate a pre-session
   *  submit with the same logic the start screen uses. */
  startBlockedReason: () => string | null;
  startSession: (options?: {
    quickChat?: { id: string; path: string };
    submittedDraft?: string;
    /** Explicit user-selected folders outside the writable checkout. */
    readRoots?: string[];
  }) => Promise<void>;
  /** Start immediately in an app-managed workspace without selecting a repo. */
  startQuickChat: () => Promise<void>;
  /** Create the explicitly approved isolated checkout, then start the chat in it. */
  confirmManagedWorktreeStart: () => Promise<void>;
  /** Leave the source checkout untouched and close the preservation decision. */
  dismissManagedWorktreeStart: () => void;
  /** Detach from the current conversation (→ welcome screen). Its live session
   *  keeps running in the background pool — reopening reattaches instantly.
   *  `force` (sign-out) tears down every live session instead. */
  endSession: (opts?: { force?: boolean }) => void;
  openConversation: (id: string) => Promise<void>;
  bindConversationProject: (id: string, path: string) => Promise<void>;
  /** Permanently remove the selected unavailable entry, then return to a clean
   * new-chat composer with no project selected. */
  cleanupUnavailableConversation: () => Promise<void>;
  /** Soft-delete: hide from the main list but keep the transcript locally and in
   *  the cloud so it can be restored. Clears the view if it's the open chat. */
  archiveConversation: (id: string) => Promise<void>;
  /** Bring an archived conversation back into the active list. */
  restoreConversation: (id: string) => Promise<void>;
  /** Permanently delete a conversation — local cache AND the cloud copy. */
  deleteConversation: (id: string) => Promise<void>;
  /** Rename a conversation; the manual title stops auto-derivation clobbering it. */
  renameConversation: (id: string, title: string) => void;
  /** Toggle one conversation in the sidebar's Shift-click selection. */
  toggleConversationSelection: (id: string) => void;
  /** Set the sidebar selection (replace). Pass an empty Set to clear. */
  setConversationSelection: (ids: Set<string>) => void;
  /** Archive every selected conversation at once. Busy ones are skipped (a
   *  notice is flashed); the rest are closed + flagged archived in the cloud. */
  archiveSelectedConversations: () => Promise<void>;
  /** Permanently delete every selected conversation at once. Busy ones are
   *  skipped (a notice is flashed); the rest are closed + deleted from the
   *  cloud. Selection is cleared afterwards. */
  deleteSelectedConversations: () => Promise<void>;
  /** Change the coding model / reasoning effort for the ACTIVE conversation.
   *  Writes a per-chat override (so other chats keep their own model), persists
   *  it, and — when a local session is live — hot-swaps that session's provider
   *  LLM (the transcript is kept; the next turn continues with full context on
   *  the new model). With no active chat, edits the global default instead. */
  updateModelSettings: (patch: { model?: string; reasoningEffort?: string }) => Promise<void>;
  /** Stage text in the composer. A timeline index makes submit replace that
   * turn and its abandoned suffix instead of appending a duplicate. */
  setComposerPrefill: (text: string | null, timelineIndex?: number) => void;
  /** Create a public read-only link for the viewed conversation + copy it. */
  shareConversation: () => Promise<void>;
  /** Revoke the viewed conversation's public link. */
  unshareConversation: () => Promise<void>;
  addFiles: (files: File[]) => Promise<void>;
  removeAttachment: (id: string) => void;
  send: (
    text: string,
    skills?: SkillReferenceBlock[],
  ) => Promise<SendOutcome>;
  /** Summarize and replace Clark Code's model-visible history without adding a user turn. */
  compactConversation: () => Promise<void>;
  continueProviderIncident: (incidentId: string) => Promise<void>;
  /** Replace one prior Clark Code user turn and rerun from the retained prefix. */
  resendFrom: (
    timelineIndex: number,
    text: string,
    skills?: SkillReferenceBlock[],
  ) => Promise<import("../core-bridge/bridge").PromptReceipt | null>;
  /** Explicitly inject one queued text-only message into the active local run. */
  steerQueued: (id: string) => Promise<void>;
  removeQueued: (id: string) => void;
  setApprovalPolicy: (mode: ApprovalPolicy) => void;
  /** Change only the account-wide default — never a chat's own level or a live
   *  session's mode. Used by the Settings picker; the composer pill (and
   *  Shift+Tab) edit the focused chat via `setApprovalPolicy`. */
  setDefaultApprovalPolicy: (mode: ApprovalPolicy) => void;
  /** Shift+Tab: advance to the next permission mode in the cycle. */
  cycleApprovalPolicy: () => void;
  setCollaborationMode: (mode: CollaborationMode) => void;
  decidePlan: (
    planId: string,
    decision: { action: "implement"; context: PlanImplementationContext } |
      { action: "continue_planning"; feedback?: string },
  ) => Promise<void>;
  setOutputStyle: (style: string) => void;
  /** Remove the session's standing goal. The transcript is kept intact; the
   *  goal stops continuing and its receipt is retired. */
  clearGoal: () => Promise<void>;
  toggleTerminal: () => void;
  setTerminalOpen: (open: boolean) => void;
  /** Make a folder the current project (seeding the next session). With no
   *  `path`, asks the OS for a folder first; cancelling the picker is a
   *  no-op. Does not force the terminal open — if one is already open it
   *  records a launch so a fresh tab roots at the folder. */
  openProjectTerminal: (path?: string) => Promise<void>;
  /** Set the project (a local folder or a remote SSH host + folder) and start
   *  its first session immediately instead of waiting for a typed prompt. */
  startNewProject: (target: NewProjectTarget) => Promise<void>;
  setMcpOpen: (open: boolean) => void;
  setSshOpen: (open: boolean, purpose?: SshOpenPurpose) => void;
  /** Open/close the "New project" chooser modal. */
  setNewProjectOpen: (open: boolean) => void;
  /** Open/close the unified Settings modal, optionally jumping to a section. */
  setSettingsOpen: (open: boolean, section?: SettingsSection) => void;
  setPaletteOpen: (open: boolean) => void;
  togglePalette: () => void;
  /** Check for, download, verify, and stage a newer version (no install yet). */
  checkForUpdate: () => Promise<UpdateCheckResult>;
  /** Drain active work, install the staged update, and relaunch. */
  applyUpdate: () => Promise<void>;
  /** Dismiss the "updated to vX" confirmation. */
  dismissJustUpdated: () => void;
  /** Clear the transient error banner. */
  dismissError: () => void;
  /** Show a transient success/info toast that auto-dismisses. */
  flashNotice: (message: string) => void;
  /** Show a transient warning toast that auto-dismisses. */
  flashWarning: (message: string) => void;
  /** Clear the transient notice toast. */
  dismissNotice: () => void;
  /** Clear the transient warning toast. */
  dismissWarning: () => void;
  /** Hide the failed/stopped terminal banner for a specific run. */
  dismissFailedRun: (runId: string) => void;
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  cancelActive: () => Promise<void>;
  resolvePermission: (option: string) => Promise<void>;
  /** `/btw` — ask a side question (forked, no main-run interruption). */
  askSideQuestion: (question: string) => Promise<void>;
  /** Dismiss the side-question overlay (and drop a still-pending answer). */
  dismissSideQuestion: () => void;
}

export type SessionSet = StoreApi<SessionState>["setState"];
export type SessionGet = () => SessionState;
