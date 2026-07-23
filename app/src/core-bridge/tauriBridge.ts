// Production bridge: thin wrapper over Tauri commands + events. The heavy
// lifting (transport, projection) happens in the native `agent-core` host.
//
// The matching Rust commands are registered in src-tauri.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  CloudTrajectoryConfig,
  CoreBridge,
  ConnectConfig,
  ProjectBranch,
  ProjectContext,
  LocalSandboxStatus,
  ProjectInstructions,
  InstalledSkillPack,
  SkillPackOperationResult,
  SkillPackScope,
  SkillCatalogChange,
  SkillCatalogSnapshot,
  SessionOptions,
} from "./bridge";
import type { Upload } from "../lib/attachments";
import {
  normalizeSnapshot,
  type WireSnapshot,
  type ClientResponse,
  type ContentBlock,
  type ProviderInfo,
  type Session,
  type Snapshot,
  type MemoryOverview,
  type CollaborationMode,
} from "./types";

export class TauriBridge implements CoreBridge {
  listProviders(): Promise<ProviderInfo[]> {
    return invoke<ProviderInfo[]>("provider_list");
  }

  connect(providerId: string, config: ConnectConfig): Promise<void> {
    return invoke("provider_connect", { providerId, config });
  }

  reconfigure(sessionId: string, config: ConnectConfig): Promise<void> {
    return invoke("provider_reconfigure", { sessionId, config });
  }

  newSession(providerId: string, options: SessionOptions, bindId?: string): Promise<Session> {
    return invoke<Session>("session_new", { providerId, options, bindId: bindId ?? null });
  }

  loadSession(providerId: string, id: string): Promise<Session> {
    return invoke<Session>("session_load", { providerId, id });
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
    return invoke("session_configure_cloud", { sessionId, config, baseSnapshot, baseRev });
  }

  updateCloudToken(token: string): Promise<void> {
    return invoke("update_cloud_token", { token });
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

  prompt(sessionId: string, blocks: ContentBlock[], attachments: Upload[] = []): Promise<void> {
    return invoke("prompt", { sessionId, blocks, attachments });
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

  listMemory(cwd: string, remote?: { ws_url: string; token: string } | null): Promise<MemoryOverview> {
    return invoke<MemoryOverview>("local_list_memory", { cwd, remote: remote ?? null });
  }

  listGlobalMemory(): Promise<MemoryOverview> {
    return invoke<MemoryOverview>("local_list_global_memory");
  }

  listFiles(cwd: string, remote?: { ws_url: string; token: string } | null): Promise<string[]> {
    return invoke<string[]>("local_list_files", { cwd, remote: remote ?? null });
  }

  listSkills(
    cwd: string,
    remote?: { ws_url: string; token: string } | null,
  ): Promise<SkillCatalogSnapshot> {
    return invoke<SkillCatalogSnapshot>("skills_list", { cwd, remote: remote ?? null });
  }

  reloadSkills(
    cwd: string,
    remote?: { ws_url: string; token: string } | null,
  ): Promise<SkillCatalogSnapshot> {
    return invoke<SkillCatalogSnapshot>("skills_reload", { cwd, remote: remote ?? null });
  }

  skillChanges(
    cwd: string,
    sinceRevision: string,
    remote?: { ws_url: string; token: string } | null,
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
    remote?: { ws_url: string; token: string } | null,
  ): Promise<ProjectInstructions | null> {
    return invoke<ProjectInstructions | null>("instructions_list", {
      cwd,
      remote: remote ?? null,
    });
  }

  listSkillPacks(
    cwd: string,
    remote?: { ws_url: string; token: string } | null,
  ): Promise<InstalledSkillPack[]> {
    return invoke<InstalledSkillPack[]>("skill_packs_list", {
      cwd,
      remote: remote ?? null,
    });
  }

  installSkillPack(
    cwd: string,
    request: { packId: string; sourcePath: string; scope: SkillPackScope },
    remote?: { ws_url: string; token: string } | null,
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
    remote?: { ws_url: string; token: string } | null,
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
    remote?: { ws_url: string; token: string } | null,
  ): Promise<ProjectContext | null> {
    return invoke<ProjectContext | null>("project_context", { cwd, remote: remote ?? null });
  }

  openPath(path: string, reveal = false): Promise<void> {
    return invoke("open_path", { path, reveal });
  }

  listProjectBranches(projectPath: string): Promise<ProjectBranch[]> {
    return invoke<ProjectBranch[]>("project_branch_list", { projectPath });
  }

  switchProjectBranch(projectPath: string, branch: string): Promise<void> {
    return invoke("project_branch_switch", { projectPath, branch });
  }

  createPermanentWorktree(projectPath: string, name: string): Promise<string> {
    return invoke<string>("project_worktree_create", { projectPath, name });
  }

  localSandboxStatus(cwd: string): Promise<LocalSandboxStatus> {
    return invoke<LocalSandboxStatus>("local_sandbox_status", { cwd });
  }

  setupLocalSandbox(cwd: string): Promise<LocalSandboxStatus> {
    return invoke<LocalSandboxStatus>("local_sandbox_setup", { cwd });
  }
}
