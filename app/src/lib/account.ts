// Account identity, Clark Code key provisioning, and shared external links.
//
// The desktop never asks the user to paste an API key: after Google sign-in it
// mints a "Clark Code" key with the user's Clark JWT and stores it. Billing
// Provisioning goes through a host-side Tauri command (no WebView CORS) and is
// gated to the desktop app + a real token. Billing policy lives in billing.ts.

import { invoke } from "@tauri-apps/api/core";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import type { AuthSession } from "./auth";
import type { CloudCreds } from "./cloudHistory";

/** clarkchat.com billing/subscription page (where users buy credits + manage
 *  their plan). Same Google account → same Clark wallet as the desktop. */
export function clarkBillingUrl(): string {
  return "https://www.clarkchat.com/billing";
}

/** Open a URL in the system browser (desktop) or a new tab (browser preview). */
export async function openExternal(url: string): Promise<void> {
  try {
    await shellOpen(url);
  } catch {
    if (typeof window !== "undefined") window.open(url, "_blank", "noopener");
  }
}

/** Ensure Clark's native encrypted file has this account's Code credential. */
export function provisionCodeKey(c: CloudCreds): Promise<{ ready: boolean }> {
  void c;
  return invoke<{ ready: boolean }>("clark_provision_code_key");
}

/** Stable server identity used for non-secret WebView cache partitioning. */
export function codeKeyAccountBinding(auth: AuthSession | null): string | null {
  const id = auth?.user.id.trim();
  if (id) return `id:${id}`;
  return null;
}

/** True only when two descriptors represent the same native Clark account. */
export function authAccountMatches(
  started: AuthSession | null,
  current: AuthSession | null,
): boolean {
  if (!started || !current) return started === null && current === null;
  const startedOwner = codeKeyAccountBinding(started);
  const currentOwner = codeKeyAccountBinding(current);
  return Boolean(startedOwner && startedOwner === currentOwner);
}
