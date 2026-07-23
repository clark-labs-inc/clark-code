// Auth — decoupled from Clark, no secrets in the repo.
//
// A sign-in yields an AuthSession that carries the user identity AND the Clark
// connection config (endpoint + token). Config comes from build-time env
// (VITE_*) or a gitignored .env.local — never hardcoded.
//
// Google sign-in is real Clark auth (server-side Better Auth). Clark only trusts
// callbacks on its own domain, and Google blocks OAuth in embedded WebViews, so
// the flow differs by build target:
//
//   • native (Tauri): `tauri-plugin-google-auth` runs the OAuth loop in the
//     system browser + a localhost loopback and returns a Google ID token; the
//     host then exchanges it with Clark's Better Auth (`/api/auth/sign-in/social`
//     with `{ idToken }`) for a Clark JWT — see the `clark_exchange_google_idtoken`
//     Tauri command. Needs a desktop Google OAuth client (VITE_GOOGLE_DESKTOP_*).
//   • web / dev browser: open the Clark `/desktop-auth` handoff page in a popup;
//     it completes the normal web sign-in (cookie is first-party there) and
//     postMessages the JWT back. Needs only the Clark auth origin.
//
// Swapping in a different identity provider only touches this file.

import { invoke } from "@tauri-apps/api/core";

export type AuthMethod = "google" | "local";

export interface AuthUser {
  /** Stable Clark account identifier. Older persisted sessions may not have it. */
  id?: string;
  name: string;
  email?: string;
  avatar?: string;
  method: AuthMethod;
}

export interface AuthSession {
  user: AuthUser;
  clark: { endpoint: string; token?: string };
  google?: {
    accessToken?: string;
    refreshToken?: string;
    expiresAt?: number;
  };
}

const SESSION_KEY = "clark.auth.session";
/** postMessage discriminator shared with the clark-ui /desktop-auth page. */
const HANDOFF_MESSAGE = "clark-desktop-auth";

const env = import.meta.env as Record<string, string | undefined>;
const config = {
  // Native Google sign-in (installed-app OAuth client; secret is non-confidential
  // per Google and embedded in the app bundle, kept out of git via .env.local).
  googleDesktopClientId: env.VITE_GOOGLE_DESKTOP_CLIENT_ID,
  googleDesktopClientSecret: env.VITE_GOOGLE_DESKTOP_CLIENT_SECRET,
  // Clark web origin hosting Better Auth + the /desktop-auth handoff page.
  // Defaults to production; override with VITE_CLARK_AUTH_ORIGIN (e.g. dev).
  clarkAuthOrigin: (env.VITE_CLARK_AUTH_ORIGIN ?? "https://www.clarkchat.com").replace(/\/+$/, ""),
  // Local dev gateway fallback (override with VITE_CLARK_ENDPOINT); the token is
  // never defaulted/committed.
  clarkEndpoint: env.VITE_CLARK_ENDPOINT ?? (import.meta.env.DEV ? "ws://localhost:8400/ws" : ""),
  devAuth: import.meta.env.DEV && env.VITE_CLARK_DEV_AUTH === "1",
};

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Derive the gateway WS endpoint from the auth origin (https→wss, +/ws). */
function wsEndpointForGoogle(): string {
  try {
    const u = new URL(config.clarkAuthOrigin);
    const proto = u.protocol === "https:" ? "wss:" : "ws:";
    return `${proto}//${u.host}/ws`;
  } catch {
    return config.clarkEndpoint;
  }
}

export function isGoogleConfigured(): boolean {
  if (!config.clarkAuthOrigin) return false;
  // Native additionally needs a desktop OAuth client; web only needs the origin.
  return isTauri() ? !!config.googleDesktopClientId : true;
}

export function loadAuthSession(): AuthSession | null {
  try {
    const devToken = import.meta.env.DEV ? env.VITE_CLARK_TOKEN : undefined;
    if (config.devAuth && config.clarkEndpoint && devToken) {
      return persist({
        user: {
          name: "Local Dev",
          email: "local-playwright@clark.local",
          method: "local",
        },
        clark: { endpoint: config.clarkEndpoint, token: devToken },
      });
    }
    const raw = localStorage.getItem(SESSION_KEY);
    if (raw) return JSON.parse(raw) as AuthSession;
    return null;
  } catch {
    return null;
  }
}

function persist(session: AuthSession): AuthSession {
  try {
    localStorage.setItem(SESSION_KEY, JSON.stringify(session));
  } catch {
    /* ignore */
  }
  return session;
}

export function signOut(): void {
  try {
    localStorage.removeItem(SESSION_KEY);
  } catch {
    /* ignore */
  }
}

// --- Google sign-in (real Clark Better Auth) --------------------------------

interface ExchangeResult {
  token: string;
  id: string;
  email: string;
  name?: string;
  image?: string;
}

interface GoogleTokenState {
  accessToken?: string;
  refreshToken?: string;
  expiresAt?: number;
}

function userFrom(p: { id?: string; email?: string; name?: string; image?: string }): AuthUser {
  return {
    id: p.id || undefined,
    name: p.name || p.email || "Google user",
    email: p.email || undefined,
    avatar: p.image || undefined,
    method: "google",
  };
}

async function exchangeGoogleIdToken(idToken: string): Promise<{ user: AuthUser; token: string }> {
  const out = await invoke<ExchangeResult>("clark_exchange_google_idtoken", {
    authOrigin: config.clarkAuthOrigin,
    idToken,
  });
  return { user: userFrom(out), token: out.token };
}

// Branded page the loopback server returns after Google redirects back. Two
// jobs: (1) look intentional instead of the plugin's bare "Go back to your
// app :)" default, and (2) bounce focus to the app via the `clark://` deep link
// so the user lands back in Clark instead of stranded on a localhost tab. The
// plugin writes this body with no Content-Type header, so lead with `<!doctype
// html>` to guarantee the browser sniffs it as HTML (and runs the redirect).
const AUTH_SUCCESS_HTML = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>Signed in to Clark Code</title>
<style>
  :root { color-scheme: light dark; }
  * { box-sizing: border-box; }
  body {
    margin: 0; min-height: 100vh; display: grid; place-items: center;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    background: #f7f5f1; color: #211f1c;
  }
  .card {
    width: min(380px, calc(100vw - 32px)); text-align: center; padding: 40px 44px;
    border: 1px solid rgba(40, 36, 32, 0.09); border-radius: 22px;
    background: #fdfcfa; box-shadow: 0 18px 46px rgba(33, 31, 28, 0.09);
  }
  .mark {
    width: 48px; height: 48px; margin: 0 auto 20px; border-radius: 16px;
    display: grid; place-items: center;
    background: #5748c7; color: #fdfcfa;
    font-family: Georgia, serif; font-weight: 600; font-size: 22px;
  }
  h1 { font-family: Georgia, serif; font-size: 1.5rem; font-weight: 600; margin: 0 0 8px; }
  p { font-size: 0.9375rem; line-height: 1.55; color: #5d5750; margin: 0 0 24px; }
  a.btn {
    display: inline-block; text-decoration: none;
    background: #5748c7; color: #fdfcfa;
    padding: 12px 20px; border-radius: 16px; font-size: 0.9375rem; font-weight: 600;
  }
  @media (prefers-color-scheme: dark) {
    body { background: #0d0d0d; color: #f5f5f4; }
    .card { background: #161616; border-color: rgba(255,255,255,0.09); box-shadow: 0 18px 56px rgba(0,0,0,0.34); }
    .mark { background: #9b8cff; color: #17131f; }
    a.btn { background: #9b8cff; color: #17131f; }
    p { color: #a8a5a0; }
  }
</style>
</head>
<body>
  <main class="card">
    <div class="mark">C</div>
    <h1>You're signed in</h1>
    <p>You can close this tab and return to Clark Code.</p>
    <a class="btn" href="clark://auth-complete">Return to Clark Code</a>
  </main>
  <script>
    // Best-effort: hand focus back to the app automatically. The button above is
    // the reliable fallback if the browser blocks the programmatic redirect.
    setTimeout(function () {
      try { window.location.replace("clark://auth-complete"); } catch (e) {}
    }, 500);
  </script>
</body>
</html>`;

/** Pull the Clark window back to the foreground after a browser round-trip. */
async function focusAppWindow(): Promise<void> {
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const w = getCurrentWindow();
    await w.unminimize();
    await w.setFocus();
  } catch {
    /* best-effort — the clark:// deep link handles focus in the packaged app */
  }
}

/** Native: plugin OAuth → Google ID token → host exchanges for a Clark JWT. */
async function googleSignInNative(): Promise<{
  user: AuthUser;
  token: string;
  google?: GoogleTokenState;
}> {
  if (!config.googleDesktopClientId) {
    throw new Error("Set VITE_GOOGLE_DESKTOP_CLIENT_ID to enable Google sign-in.");
  }
  const { signIn } = await import("@choochmeque/tauri-plugin-google-auth-api");
  const res = await signIn({
    clientId: config.googleDesktopClientId,
    clientSecret: config.googleDesktopClientSecret,
    scopes: ["openid", "email", "profile"],
    // Bind the loopback on 127.0.0.1 — matching the plugin's actual bind host —
    // so the browser doesn't dead-end on an IPv6 `localhost` nothing listens on;
    // the empty port lets the OS pick a free one.
    redirectUri: "http://127.0.0.1",
    // Branded, self-explaining success page that also deep-links focus back to
    // the app, instead of the plugin's bare default text.
    successHtmlResponse: AUTH_SUCCESS_HTML,
  });
  const idToken = res.idToken;
  if (!idToken) throw new Error("Google did not return an ID token.");
  const out = await exchangeGoogleIdToken(idToken);
  await focusAppWindow();
  return {
    ...out,
    google: {
      accessToken: res.accessToken,
      refreshToken: res.refreshToken,
      expiresAt: res.expiresAt,
    },
  };
}

/** Web/dev: popup the Clark /desktop-auth handoff page; it postMessages the JWT. */
async function googleSignInWeb(): Promise<{
  user: AuthUser;
  token: string;
  google?: GoogleTokenState;
}> {
  const origin = config.clarkAuthOrigin;
  const popup = window.open(
    `${origin}/desktop-auth`,
    "clark-google-signin",
    "width=480,height=680,menubar=no,toolbar=no",
  );
  if (!popup) {
    throw new Error("Popup blocked. Allow popups for this site to sign in with Google.");
  }
  return new Promise((resolve, reject) => {
    let settled = false;
    const finish = (fn: () => void) => {
      if (settled) return;
      settled = true;
      window.removeEventListener("message", onMessage);
      window.clearInterval(poll);
      window.clearTimeout(timeout);
      try {
        popup.close();
      } catch {
        /* ignore */
      }
      fn();
    };
    const onMessage = (e: MessageEvent) => {
      if (e.origin !== origin) return;
      const d = e.data as { type?: string; token?: string; user?: Record<string, string> };
      if (!d || d.type !== HANDOFF_MESSAGE) return;
      if (!d.token) {
        finish(() => reject(new Error("Sign-in did not return a token.")));
        return;
      }
      finish(() => resolve({ user: userFrom(d.user ?? {}), token: d.token! }));
    };
    const poll = window.setInterval(() => {
      if (popup.closed) finish(() => reject(new Error("Sign-in window was closed.")));
    }, 500);
    const timeout = window.setTimeout(
      () => finish(() => reject(new Error("Sign-in timed out. Please try again."))),
      300_000,
    );
    window.addEventListener("message", onMessage);
  });
}

export async function signInWithGoogle(): Promise<AuthSession> {
  if (!config.clarkAuthOrigin) {
    throw new Error("Google sign-in isn't configured. Set VITE_CLARK_AUTH_ORIGIN.");
  }
  const { user, token, google } = isTauri() ? await googleSignInNative() : await googleSignInWeb();
  return persist({
    user,
    clark: { endpoint: wsEndpointForGoogle(), token },
    google,
  });
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

export async function refreshAuthSession(session: AuthSession): Promise<AuthSession | null> {
  if (!isTauri() || session.user.method !== "google") return null;
  if (!config.googleDesktopClientId) return null;
  const refresh = session.google?.refreshToken;
  if (!refresh) return null;
  try {
    const { refreshToken: refreshGoogleToken } = await import(
      "@choochmeque/tauri-plugin-google-auth-api"
    );
    const refreshed = await refreshGoogleToken({
      refreshToken: refresh,
      clientId: config.googleDesktopClientId,
      clientSecret: config.googleDesktopClientSecret,
      scopes: ["openid", "email", "profile"],
    });
    if (!refreshed.idToken) return null;
    const exchanged = await exchangeGoogleIdToken(refreshed.idToken);
    return persist({
      user: exchanged.user,
      clark: { endpoint: session.clark.endpoint || wsEndpointForGoogle(), token: exchanged.token },
      google: {
        accessToken: refreshed.accessToken,
        refreshToken: refreshed.refreshToken || refresh,
        expiresAt: refreshed.expiresAt,
      },
    });
  } catch {
    return null;
  }
}
