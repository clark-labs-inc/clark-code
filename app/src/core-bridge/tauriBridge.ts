// Production bridge: thin wrapper over Tauri commands + events. The heavy
// lifting (transport, projection) happens in the native `agent-core` host.
//
// The matching Rust commands are registered in src-tauri.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { productRequest } from "../product/productBridge";
import type {
  CloudTrajectoryConfig,
  ComputerUseActionReceipt,
  ComputerUseApprovalSnapshot,
  ComputerUsePermissionStatus,
  ComputerUsePlatformStatus,
  CoreBridge,
  ConnectConfig,
  ManagedWorktree,
  ManagedWorktreeBranchReceipt,
  ManagedWorktreeCleanupReceipt,
  ManagedWorktreeRequest,
  ProjectBranch,
  ProjectContext,
  ProjectWorktreeTransitionPlan,
  LocalSandboxStatus,
  PromptReceipt,
  ProjectDirectory,
  ProjectInstructions,
  RemoteWorkerTarget,
  InstalledSkillPack,
  SkillPackOperationResult,
  SkillPackScope,
  SkillCatalogChange,
  SkillCatalogSnapshot,
  SessionOpenRequest,
} from "./bridge";
import type { Upload } from "../lib/attachments";
import {
  normalizeSnapshot,
  type WireSnapshot,
  type ClientResponse,
  type ContentBlock,
  type ProviderInfo,
  type SpecialistProjectionPublished,
  type SpecialistCatalogAttestation,
  type Session,
  type Snapshot,
  type MemoryOverview,
  type SecurityScanRecord,
  type CollaborationMode,
} from "./types";

export class TauriBridge implements CoreBridge {
  listProviders(): Promise<ProviderInfo[]> {
    return invoke<ProviderInfo[]>("provider_list");
  }

  listSpecialistCatalog(): Promise<SpecialistCatalogAttestation> {
    return productRequest<SpecialistCatalogAttestation>("specialist.catalog");
  }

  prepareQuickChatWorkspace(id?: string) {
    return invoke<import("./bridge").QuickChatWorkspace>("prepare_quick_chat_workspace", {
      id: id ?? null,
    });
  }

  reconfigure(sessionId: string, config: ConnectConfig): Promise<void> {
    return invoke("provider_reconfigure", { sessionId, config });
  }

  addReadRoots(sessionId: string, roots: string[]): Promise<void> {
    return invoke("provider_add_read_roots", { sessionId, roots });
  }

  removeReadRoots(sessionId: string, roots: string[]): Promise<void> {
    return invoke("provider_remove_read_roots", { sessionId, roots });
  }

  openSession(
    providerId: string,
    config: ConnectConfig,
    request: SessionOpenRequest,
  ): Promise<Session> {
    const wireRequest = request.kind === "new"
      ? { kind: "new", options: request.options, bind_id: request.bindId ?? null }
      : request;
    return invoke<Session>("session_open", { providerId, config, request: wireRequest });
  }

  closeSession(sessionId: string): Promise<void> {
    return invoke("session_close", { sessionId });
  }

  configureCloudTrajectory(
    sessionId: string,
    config: CloudTrajectoryConfig,
    baseSnapshot: Snapshot,
    baseRev: number,
  ): Promise<void> {
    return invoke("session_configure_cloud", {
      sessionId,
      config,
      baseSnapshot,
      baseRev,
    });
  }

  onCloudAuthExpired(handler: () => void): () => void {
    const unlisten = listen("cloud-auth-expired", () => handler());
    return () => {
      void unlisten.then((fn) => fn());
    };
  }

  onCloudSyncWarning(handler: (message: string) => void): () => void {
    const unlisten = listen<string>("cloud-sync-warning", (event) => handler(event.payload));
    return () => {
      void unlisten.then((fn) => fn());
    };
  }

  onSpecialistProjectionPublished(
    handler: (receipt: SpecialistProjectionPublished) => void,
  ): () => void {
    const unlisten = listen<SpecialistProjectionPublished>(
      "specialist-projection-published",
      (event) => handler(event.payload),
    );
    return () => {
      void unlisten.then((fn) => fn());
    };
  }

  onCloudConversationDeleted(handler: (conversationId: string) => void): () => void {
    const unlisten = listen<string>("cloud-conversation-deleted", (event) => handler(event.payload));
    return () => {
      void unlisten.then((fn) => fn());
    };
  }

  prompt(
    sessionId: string,
    blocks: ContentBlock[],
    attachments: Upload[] = [],
  ): Promise<PromptReceipt> {
    return invoke("prompt", { sessionId, blocks, attachments });
  }

  resumeSavedProgress(sessionId: string): Promise<PromptReceipt> {
    return invoke("prompt", {
      sessionId,
      blocks: [{
        type: "text",
        text: "Continue from the saved progress. Re-read current state, do not repeat completed writes, and finish the task.",
      }],
      attachments: [],
      internalResume: true,
    });
  }

  compact(sessionId: string): Promise<void> {
    return invoke("compact", { sessionId });
  }

  steer(sessionId: string, blocks: ContentBlock[]): Promise<void> {
    return invoke("steer", { sessionId, blocks });
  }

  cancel(sessionId: string, runId: string): Promise<void> {
    return invoke("cancel", { sessionId, runId });
  }

  respond(sessionId: string, response: ClientResponse): Promise<void> {
    return invoke("respond", { sessionId, response });
  }

  setMode(sessionId: string, mode: string): Promise<void> {
    return invoke("set_mode", { sessionId, mode });
  }

  setCollaborationMode(sessionId: string, mode: CollaborationMode): Promise<void> {
    return invoke("set_collaboration_mode", { sessionId, mode });
  }

  setOutputStyle(sessionId: string, style: string): Promise<void> {
    return invoke("set_output_style", { sessionId, style });
  }

  sideQuestion(sessionId: string, question: string): Promise<string> {
    return invoke<string>("side_question", { sessionId, question });
  }

  subscribe(handler: (snapshot: Snapshot) => void): () => void {
    const unlisten = listen<WireSnapshot>("snapshot", (event) => {
      handler(normalizeSnapshot(event.payload));
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }

  listMemory(sessionId: string): Promise<MemoryOverview> {
    return invoke<MemoryOverview>("local_list_memory", { sessionId });
  }

  listGlobalMemory(): Promise<MemoryOverview> {
    return invoke<MemoryOverview>("local_list_global_memory");
  }

  listFiles(cwd: string, remote?: RemoteWorkerTarget | null): Promise<string[]> {
    return invoke<string[]>("local_list_files", { cwd, remote: remote ?? null });
  }

  listSiblingDirectories(
    cwd: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<ProjectDirectory[]> {
    return invoke<ProjectDirectory[]>("local_list_sibling_directories", {
      cwd,
      remote: remote ?? null,
    });
  }

  listSecurityScans(
    cwd: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<SecurityScanRecord[]> {
    return invoke<SecurityScanRecord[]>("local_list_security_scans", {
      cwd,
      remote: remote ?? null,
    });
  }

  listSkills(
    cwd: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<SkillCatalogSnapshot> {
    return invoke<SkillCatalogSnapshot>("skills_list", { cwd, remote: remote ?? null });
  }

  reloadSkills(
    cwd: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<SkillCatalogSnapshot> {
    return invoke<SkillCatalogSnapshot>("skills_reload", { cwd, remote: remote ?? null });
  }

  skillChanges(
    cwd: string,
    sinceRevision: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<SkillCatalogChange> {
    return invoke<SkillCatalogChange>("skills_changes", {
      cwd,
      sinceRevision,
      remote: remote ?? null,
    });
  }

  onSkillsChanged(handler: (snapshot: SkillCatalogSnapshot) => void): () => void {
    const unlisten = listen<SkillCatalogSnapshot>("skill-catalog-changed", (event) => {
      handler(event.payload);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }

  listInstructions(
    cwd: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<ProjectInstructions | null> {
    return invoke<ProjectInstructions | null>("instructions_list", {
      cwd,
      remote: remote ?? null,
    });
  }

  listSkillPacks(
    cwd: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<InstalledSkillPack[]> {
    return invoke<InstalledSkillPack[]>("skill_packs_list", {
      cwd,
      remote: remote ?? null,
    });
  }

  installSkillPack(
    cwd: string,
    request: { packId: string; sourcePath: string; scope: SkillPackScope },
    remote?: RemoteWorkerTarget | null,
  ): Promise<SkillPackOperationResult> {
    return invoke<SkillPackOperationResult>("skill_pack_install", {
      cwd,
      request,
      remote: remote ?? null,
    });
  }

  uninstallSkillPack(
    cwd: string,
    packId: string,
    scope: SkillPackScope,
    remote?: RemoteWorkerTarget | null,
  ): Promise<SkillPackOperationResult> {
    return invoke<SkillPackOperationResult>("skill_pack_uninstall", {
      cwd,
      packId,
      scope,
      remote: remote ?? null,
    });
  }

  projectContext(
    cwd: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<ProjectContext | null> {
    return invoke<ProjectContext | null>("project_context", { cwd, remote: remote ?? null });
  }

  openPath(path: string, reveal = false): Promise<void> {
    return invoke("open_path", { path, reveal });
  }

  listProjectBranches(
    projectPath: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<ProjectBranch[]> {
    return invoke<ProjectBranch[]>("project_branch_list", { projectPath, remote: remote ?? null });
  }

  switchProjectBranch(
    projectPath: string,
    branch: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<void> {
    return invoke("project_branch_switch", { projectPath, branch, remote: remote ?? null });
  }

  createPermanentWorktree(
    projectPath: string,
    name: string,
    remote?: RemoteWorkerTarget | null,
  ): Promise<string> {
    return invoke<string>("project_worktree_create", { projectPath, name, remote: remote ?? null });
  }

  planProjectWorktree(
    projectPath: string,
    targetBranch?: string | null,
  ): Promise<ProjectWorktreeTransitionPlan> {
    return invoke<ProjectWorktreeTransitionPlan>("project_worktree_transition_plan", {
      projectPath,
      targetBranch: targetBranch ?? null,
    });
  }

  createManagedWorktree(
    projectPath: string,
    request: ManagedWorktreeRequest,
  ): Promise<ManagedWorktree> {
    return invoke<ManagedWorktree>("project_managed_worktree_create", { projectPath, request });
  }

  listManagedWorktrees(projectPath: string): Promise<ManagedWorktree[]> {
    return invoke<ManagedWorktree[]>("project_managed_worktree_list", { projectPath });
  }

  cleanupManagedWorktree(
    projectPath: string,
    id: string,
  ): Promise<ManagedWorktreeCleanupReceipt> {
    return invoke<ManagedWorktreeCleanupReceipt>("project_managed_worktree_cleanup", {
      projectPath,
      id,
    });
  }

  saveManagedWorktreeBranch(
    projectPath: string,
    id: string,
  ): Promise<ManagedWorktreeBranchReceipt> {
    return invoke<ManagedWorktreeBranchReceipt>("project_managed_worktree_save_branch", {
      projectPath,
      id,
    });
  }

  localSandboxStatus(cwd: string): Promise<LocalSandboxStatus> {
    return invoke<LocalSandboxStatus>("local_sandbox_status", { cwd });
  }

  setupLocalSandbox(cwd: string): Promise<LocalSandboxStatus> {
    return invoke<LocalSandboxStatus>("local_sandbox_setup", { cwd });
  }

  computerUsePlatformStatus(): Promise<ComputerUsePlatformStatus> {
    return invoke<ComputerUsePlatformStatus>("computer_use_platform_status");
  }

  requestComputerUsePermissions(): Promise<ComputerUsePermissionStatus> {
    return invoke<ComputerUsePermissionStatus>("computer_use_request_permissions");
  }

  computerUseApprovalSnapshot(): Promise<ComputerUseApprovalSnapshot> {
    return invoke<ComputerUseApprovalSnapshot>("computer_use_approval_snapshot");
  }

  revokeComputerUseApproval(identityKey: string): Promise<ComputerUseApprovalSnapshot> {
    return invoke<ComputerUseApprovalSnapshot>("computer_use_revoke_approval", { identityKey });
  }

  revokeAllComputerUseApprovals(): Promise<ComputerUseApprovalSnapshot> {
    return invoke<ComputerUseApprovalSnapshot>("computer_use_revoke_all_approvals");
  }

  recentComputerUseReceipts(): Promise<ComputerUseActionReceipt[]> {
    return invoke<ComputerUseActionReceipt[]>("computer_use_recent_receipts");
  }
}
