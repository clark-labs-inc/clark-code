// Account identity, native model-key provisioning, and shared external links.
//
// The desktop never asks the user to paste an API key: after Google sign-in it
// mints a model key with the user's session and stores it. Provisioning goes
// through a host-side command so credentials never cross the WebView boundary.

import { productRequest } from "../product/productBridge";
export { openExternal } from "./externalLinks";
import type { AuthSession } from "./auth";
import type { CloudCreds } from "./cloudHistory";

/** Ensure the agent's native encrypted file has this account's Code credential. */
export function provisionCodeKey(c: CloudCreds): Promise<{ ready: boolean }> {
  void c;
  return productRequest<{ ready: boolean }>("account.ensure_model_key");
}

/** Stable server identity used for non-secret WebView cache partitioning. */
export function codeKeyAccountBinding(auth: AuthSession | null): string | null {
  const id = auth?.user.id.trim();
  if (id) return `id:${id}`;
  return null;
}

/** True only when two descriptors represent the same native product account. */
export function authAccountMatches(
  started: AuthSession | null,
  current: AuthSession | null,
): boolean {
  if (!started || !current) return started === null && current === null;
  const startedOwner = codeKeyAccountBinding(started);
  const currentOwner = codeKeyAccountBinding(current);
  return Boolean(startedOwner && startedOwner === currentOwner);
}
