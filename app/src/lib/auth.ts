// Native Clark Desktop authentication.
//
// Google ID/access/refresh tokens and the Clark bearer exist only in the
// Tauri host. The WebView receives this non-secret account descriptor and uses
// native commands that resolve the current account atomically.

import { invoke } from "@tauri-apps/api/core";

export type AuthMethod = "google" | "local";

export interface AuthUser {
  /** Stable, server-validated Clark account identifier. */
  id: string;
  name: string;
  email?: string;
  avatar?: string;
  method: AuthMethod;
}

export interface AuthSession {
  user: AuthUser;
}

const DEV_SESSION_KEY = "clark.desktop.dev-account";
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
      Object.hasOwn(descriptor, "clark")
    ) return null;
    return descriptor as AuthSession;
  } catch {
    return null;
  }
}

/** Load the native-retained account before stores derive account partitions. */
export async function initializeAuthSession(): Promise<void> {
  if (isTauri()) {
    cachedSession = await invoke<AuthSession | null>("clark_account_load");
    localStorage.removeItem(DEV_SESSION_KEY);
    return;
  }

  // Browser-only component tests and previews have no native credential host.
  // This descriptor contains no usable bearer and never ships in Tauri builds.
  if (import.meta.env.DEV && import.meta.env.VITE_CLARK_DEV_AUTH === "1") {
    cachedSession = parseDescriptor(localStorage.getItem(DEV_SESSION_KEY)) ?? {
      user: {
        id: "local-playwright",
        name: "Local Dev",
        email: "local-playwright@clark.local",
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

export async function signInWithGoogle(): Promise<AuthSession> {
  if (!isTauri()) throw new Error("Google sign-in is available in Clark Desktop.");
  const descriptor = await invoke<AuthSession>("clark_google_sign_in");
  cachedSession = descriptor;
  return descriptor;
}

export async function signOut(): Promise<void> {
  if (isTauri()) await invoke("clark_sign_out");
  localStorage.removeItem(DEV_SESSION_KEY);
  cachedSession = null;
}

export function isAuthExpiredError(error: unknown): boolean {
  const message = String(error);
  return (
    /\b401\b/.test(message) ||
    /Unauthorized/i.test(message) ||
    /ExpiredSignature/i.test(message) ||
    /JWT validation failed/i.test(message)
  );
}

export async function refreshAuthSession(_session: AuthSession): Promise<AuthSession | null> {
  if (!isTauri()) return null;
  try {
    const descriptor = await invoke<AuthSession>("clark_refresh_cloud_session");
    cachedSession = descriptor;
    return descriptor;
  } catch {
    return null;
  }
}
