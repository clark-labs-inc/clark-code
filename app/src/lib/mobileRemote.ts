// Clark Code mobile remote control helpers.
//
// The desktop app talks to Clark through Tauri commands so auth and CORS stay
// host-side. These helpers intentionally do not execute commands themselves;
// they only register this host, poll durable mobile-originated commands, and
// record host receipts.

import { invoke } from "@tauri-apps/api/core";
import type { CloudCreds } from "./cloudHistory";

export type CodeRemoteProjectKind = "local" | "ssh";

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
  projects: CodeRemoteProjectRegistration[];
}

export interface CodeRemoteCommand {
  command_id: string;
  host_id: string;
  project_id?: string | null;
  desktop_id?: string | null;
  command_type: "start_session" | "send_message" | "cancel_run" | "resolve_permission";
  request: Record<string, unknown>;
  response?: Record<string, unknown> | null;
  status: "pending" | "delivered" | "accepted" | "completed" | "failed" | "rejected";
  base_rev?: number | null;
  delivered_at?: string | null;
  acked_at?: string | null;
  created_at: string;
  updated_at: string;
}

export interface PollCodeRemoteCommandsResponse {
  protocol_version: number;
  commands: CodeRemoteCommand[];
}

export interface CodeRemoteCommandReceipt {
  protocol_version: number;
  command: CodeRemoteCommand;
}

export async function registerCodeRemoteHost(
  creds: CloudCreds,
  registration: CodeRemoteHostRegistration,
): Promise<unknown> {
  return invoke("desktop_code_host_upsert", {
    endpoint: creds.endpoint,
    token: creds.token,
    hostId: registration.hostId,
    displayName: registration.displayName,
    osName: registration.os,
    arch: registration.arch,
    appVersion: registration.appVersion,
    projects: registration.projects,
  });
}

export async function pollCodeRemoteCommands(
  creds: CloudCreds,
  hostId: string,
  limit = 20,
  waitMs = 0,
): Promise<PollCodeRemoteCommandsResponse> {
  return invoke<PollCodeRemoteCommandsResponse>("desktop_code_command_poll", {
    endpoint: creds.endpoint,
    token: creds.token,
    hostId,
    limit,
    waitMs,
  });
}

export async function ackCodeRemoteCommand(
  creds: CloudCreds,
  hostId: string,
  commandId: string,
  status: "accepted" | "completed" | "failed" | "rejected",
  response: Record<string, unknown> = {},
): Promise<CodeRemoteCommandReceipt> {
  return invoke<CodeRemoteCommandReceipt>("desktop_code_command_ack", {
    endpoint: creds.endpoint,
    token: creds.token,
    commandId,
    hostId,
    status,
    response,
  });
}
