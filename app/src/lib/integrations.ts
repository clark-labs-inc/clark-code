import { invoke, isTauri } from "@tauri-apps/api/core";

export interface IntegrationManifest {
  id: string;
  name: string;
  description: string;
  capabilities: string[];
  experimental: boolean;
}

export interface IntegrationAvailability {
  supported: boolean;
  detail: string;
}

export interface IntegrationConversation { id: string; self_address: string }
export interface IntegrationMessage { id: string; text: string; from_me: boolean; unix_seconds: number }
type Request =
  | { action: "catalog" }
  | { action: "status" | "connect" | "disable_read_tool" | "revoke"; id: string }
  | { action: "open_settings" }
  | { action: "select"; id: string; conversation_id: string }
  | { action: "enable_read_tool"; id: string; message_ids: string[] };

// This intentionally has no browser/devbridge mock that looks like a working
// Messages connection. Browser/devbridge previews never emulate native access.
export async function integrationRequest<T>(request: Request, sessionId: string | null = null): Promise<T> {
  if (!isTauri()) throw new Error("Native integrations require the Clark Code desktop app. Browser previews cannot access Messages.");
  return invoke<T>("integration_request", { request, sessionId });
}
