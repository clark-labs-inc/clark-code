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

export type AuthMethod = "google";

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
  // Local dev gateway fallback (override with VITE_CLARK_ENDPOINT); the token is
  // never defaulted/committed.
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
    font-family: "Inter", -apple-system, system-ui, "Segoe UI", sans-serif;
    background: #ffffff; color: #14141a;
  }
  .card { text-align: center; padding: 40px 44px; max-width: 380px; }
  .mark {
    width: 44px; height: 44px; margin: 0 auto 20px; border-radius: 12px;
    display: grid; place-items: center;
    background: #14141a; color: #ffffff; font-weight: 600; font-size: 20px;
  }
  h1 { font-size: 1.0625rem; font-weight: 600; margin: 0 0 8px; }
  p { font-size: 0.875rem; line-height: 1.5; color: #52525a; margin: 0 0 24px; }
  a.btn {
    display: inline-block; text-decoration: none;
    background: #14141a; color: #ffffff;
    padding: 10px 20px; border-radius: 7px; font-size: 0.875rem; font-weight: 500;
  }
  @media (prefers-color-scheme: dark) {
    body { background: #0a0a0a; color: #f4f4f3; }
    .mark { background: #f4f4f3; color: #0a0a0a; }
    p { color: #a0a09d; }
    a.btn { background: #f4f4f3; color: #0a0a0a; }
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
  const out = await invoke<ExchangeResult>("clark_exchange_google_idtoken", {
    authOrigin: config.clarkAuthOrigin,
    idToken,
  });
  await focusAppWindow();
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
