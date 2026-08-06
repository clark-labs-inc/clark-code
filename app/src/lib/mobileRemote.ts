// Clark Code mobile remote control helpers.
//
// The desktop app talks to Clark through Tauri commands so auth and CORS stay
// host-side. These helpers intentionally do not execute commands themselves;
// they only register this host, poll durable mobile-originated commands, and
// record host receipts.

import { invoke } from "@tauri-apps/api/core";
import type { CloudCreds } from "./cloudHistory";

export type CodeRemoteProjectKind = "local" | "ssh";
export const CODE_REMOTE_PROTOCOL_VERSION = 2;
export const CODE_REMOTE_CAPABILITIES: CodeRemoteCommand["command_type"][] = [
  "start_session",
  "send_message",
  "cancel_run",
  "resolve_permission",
  "steer_run",
  "compact_conversation",
  "edit_and_resend",
];

export interface CodeRemoteProjectRegistration {
  id: string;
  kind: CodeRemoteProjectKind;
  display_name: string;
  root: string;
  ssh_alias?: string | null;
  trusted: boolean;
  repository_fingerprint?: string | null;
}

export interface CodeRemoteHostRegistration {
  hostId: string;
  displayName: string;
  os: string;
  arch: string;
  appVersion: string;
  protocolVersion: number;
  capabilities: CodeRemoteCommand["command_type"][];
  projects: CodeRemoteProjectRegistration[];
}

export interface CodeRemoteCommand {
  command_id: string;
  host_id: string;
  project_id?: string | null;
  desktop_id?: string | null;
  command_type:
    | "start_session"
    | "send_message"
    | "cancel_run"
    | "resolve_permission"
    | "steer_run"
    | "compact_conversation"
    | "edit_and_resend";
  request: Record<string, unknown>;
  response?: Record<string, unknown> | null;
  status: "pending" | "delivered" | "accepted" | "completed" | "failed" | "rejected";
  base_rev?: number | null;
  delivered_at?: string | null;
  acked_at?: string | null;
  accepted_at?: string | null;
  settled_at?: string | null;
  claim_instance_id?: string | null;
  claim_expires_at?: string | null;
  created_at: string;
  updated_at: string;
  timing?: {
    delivery_ms?: number;
    acceptance_ms?: number;
    execution_receipt_ms?: number;
    total_receipt_ms?: number;
    delivery_slo_met?: boolean;
    execution_receipt_slo_met?: boolean;
  };
}

export interface PollCodeRemoteCommandsResponse {
  protocol_version: number;
  commands: CodeRemoteCommand[];
}

export interface CodeRemoteCommandReceipt {
  protocol_version: number;
  command: CodeRemoteCommand;
}

export interface DownloadedCodeRemoteAttachment {
  filename: string;
  content_type: string;
  size_bytes: number;
  data_base64: string;
}

export async function registerCodeRemoteHost(
  _creds: CloudCreds,
  registration: CodeRemoteHostRegistration,
): Promise<unknown> {
  return invoke("desktop_code_host_upsert", {
    hostId: registration.hostId,
    displayName: registration.displayName,
    osName: registration.os,
    arch: registration.arch,
    appVersion: registration.appVersion,
    protocolVersion: registration.protocolVersion,
    capabilities: registration.capabilities,
    projects: registration.projects,
  });
}

export async function pollCodeRemoteCommands(
  _creds: CloudCreds,
  hostId: string,
  instanceId: string,
  limit = 20,
  waitMs = 0,
): Promise<PollCodeRemoteCommandsResponse> {
  return invoke<PollCodeRemoteCommandsResponse>("desktop_code_command_poll", {
    hostId,
    instanceId,
    limit,
    waitMs,
  });
}

export async function ackCodeRemoteCommand(
  _creds: CloudCreds,
  hostId: string,
  instanceId: string,
  commandId: string,
  status: "accepted" | "completed" | "failed" | "rejected",
  response: Record<string, unknown> = {},
): Promise<CodeRemoteCommandReceipt> {
  return invoke<CodeRemoteCommandReceipt>("desktop_code_command_ack", {
    commandId,
    hostId,
    instanceId,
    status,
    response,
  });
}

export async function downloadCodeRemoteAttachment(
  _creds: CloudCreds,
  commandId: string,
  attachmentId: string,
): Promise<DownloadedCodeRemoteAttachment> {
  return invoke<DownloadedCodeRemoteAttachment>("desktop_code_attachment_download", {
    commandId,
    attachmentId,
  });
}
