// Product-provided native authentication.
//
// Google ID/access/refresh tokens and the agent bearer exist only in the
// Tauri host. The WebView receives this non-secret account descriptor and uses
// native commands that resolve the current account atomically.

import { productRequest } from "../product/productBridge";
import { productModule } from "../product/productModule";

export type AuthMethod = "google" | "local";

export interface AuthUser {
  /** Stable, server-validated product account identifier. */
  id: string;
  name: string;
  email?: string;
  avatar?: string;
  method: AuthMethod;
}

export interface AuthSession {
  user: AuthUser;
  connection?: "ready" | "offline" | "reconnect_required";
}

const DEV_SESSION_KEY = "agent-desktop.dev-account";
let cachedSession: AuthSession | null = null;

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function parseDescriptor(raw: string | null): AuthSession | null {
  if (!raw) return null;
  try {
    const descriptor = JSON.parse(raw) as Partial<AuthSession>;
    if (
      !descriptor.user ||
      typeof descriptor.user.id !== "string" ||
      !descriptor.user.id ||
      typeof descriptor.user.name !== "string" ||
      (descriptor.user.method !== "google" && descriptor.user.method !== "local") ||
      (descriptor.connection !== undefined
        && descriptor.connection !== "ready"
        && descriptor.connection !== "offline"
        && descriptor.connection !== "reconnect_required") ||
      ["token", "bearer", "credentials"].some((field) => Object.hasOwn(descriptor, field))
    ) return null;
    return descriptor as AuthSession;
  } catch {
    return null;
  }
}

/** Load the native-retained account before stores derive account partitions. */
export async function initializeAuthSession(): Promise<void> {
  if (isTauri()) {
    if (!productModule().authRequired) {
      cachedSession = null;
      return;
    }
    cachedSession = await productRequest<AuthSession | null>("account.load");
    localStorage.removeItem(DEV_SESSION_KEY);
    return;
  }

  // Browser-only component tests and previews have no native credential host.
  // This descriptor contains no usable bearer and never ships in Tauri builds.
  if (import.meta.env.DEV && import.meta.env.VITE_PRODUCT_DEV_AUTH === "1") {
    cachedSession = parseDescriptor(localStorage.getItem(DEV_SESSION_KEY)) ?? {
      user: {
        id: "local-playwright",
        name: "Local Dev",
        email: "local-playwright@example.test",
        method: "local",
      },
    };
    localStorage.setItem(DEV_SESSION_KEY, JSON.stringify(cachedSession));
  } else {
    cachedSession = parseDescriptor(localStorage.getItem(DEV_SESSION_KEY));
  }
}

export function loadAuthSession(): AuthSession | null {
  return cachedSession;
}

export function authConnection(session: AuthSession | null): NonNullable<AuthSession["connection"]> | null {
  return session ? session.connection ?? "ready" : null;
}

export function markAuthReconnectRequired(session: AuthSession): AuthSession {
  const reconnecting = { ...session, connection: "reconnect_required" as const };
  cachedSession = reconnecting;
  return reconnecting;
}

export async function signInWithGoogle(): Promise<AuthSession> {
  if (!isTauri()) throw new Error("Google sign-in requires the native desktop app.");
  const descriptor = await productRequest<AuthSession>("account.sign_in");
  cachedSession = descriptor;
  return descriptor;
}

export async function signOut(): Promise<void> {
  if (isTauri()) await productRequest("account.sign_out");
  localStorage.removeItem(DEV_SESSION_KEY);
  cachedSession = null;
}

export async function refreshAuthSession(
  session: AuthSession,
  sessionStillActive: () => boolean,
): Promise<AuthSession> {
  if (!isTauri()) throw new Error("Account refresh requires the native desktop app.");
  const descriptor = await productRequest<AuthSession>("account.refresh");
  if (descriptor.user.id !== session.user.id) {
    throw new Error("Clark refreshed a different account than the active session.");
  }
  if (!sessionStillActive()) {
    throw new Error("The active account changed while Clark refreshed access.");
  }
  cachedSession = descriptor;
  return descriptor;
}
