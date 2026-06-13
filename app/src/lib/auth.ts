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

export type AuthMethod = "demo" | "google";

export interface AuthUser {
  name: string;
  email?: string;
  avatar?: string;
  method: AuthMethod;
}

export interface AuthSession {
  user: AuthUser;
  clark: { endpoint: string; token?: string };
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
  // Demo/bypass connection (local stack). Endpoint defaults to the local gateway
  // in dev; the token is never defaulted/committed.
  clarkEndpoint: env.VITE_CLARK_ENDPOINT ?? (import.meta.env.DEV ? "ws://localhost:8400/ws" : ""),
  clarkToken: env.VITE_CLARK_TOKEN,
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
    const raw = localStorage.getItem(SESSION_KEY);
    return raw ? (JSON.parse(raw) as AuthSession) : null;
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

export function signInDemo(): AuthSession {
  return persist({
    user: { name: "Demo", method: "demo" },
    clark: { endpoint: config.clarkEndpoint, token: config.clarkToken },
  });
}

// --- Google sign-in (real Clark Better Auth) --------------------------------

interface ExchangeResult {
  token: string;
  id: string;
  email: string;
  name?: string;
  image?: string;
}

function userFrom(p: { email?: string; name?: string; image?: string }): AuthUser {
  return {
    name: p.name || p.email || "Google user",
    email: p.email || undefined,
    avatar: p.image || undefined,
    method: "google",
  };
}

/** Native: plugin OAuth → Google ID token → host exchanges for a Clark JWT. */
async function googleSignInNative(): Promise<{ user: AuthUser; token: string }> {
  if (!config.googleDesktopClientId) {
    throw new Error("Set VITE_GOOGLE_DESKTOP_CLIENT_ID to enable Google sign-in.");
  }
  const [{ signIn }, { invoke }] = await Promise.all([
    import("@choochmeque/tauri-plugin-google-auth-api"),
    import("@tauri-apps/api/core"),
  ]);
  const res = await signIn({
    clientId: config.googleDesktopClientId,
    clientSecret: config.googleDesktopClientSecret,
    scopes: ["openid", "email", "profile"],
  });
  const idToken = res.idToken;
  if (!idToken) throw new Error("Google did not return an ID token.");
  const out = await invoke<ExchangeResult>("clark_exchange_google_idtoken", {
    authOrigin: config.clarkAuthOrigin,
    idToken,
  });
  return { user: userFrom(out), token: out.token };
}

/** Web/dev: popup the Clark /desktop-auth handoff page; it postMessages the JWT. */
async function googleSignInWeb(): Promise<{ user: AuthUser; token: string }> {
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
  const { user, token } = isTauri() ? await googleSignInNative() : await googleSignInWeb();
  return persist({
    user,
    clark: { endpoint: wsEndpointForGoogle(), token },
  });
}
