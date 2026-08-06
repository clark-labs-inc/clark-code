#!/usr/bin/env node

import { createHash } from "node:crypto";
import { statSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { readIgnoredEnv } from "./utm-unattended-config.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const DEFAULT_ENV_PATH = path.join(repoDir, ".env");
const DEFAULT_AUTH_ORIGIN = "https://www.clarkchat.com";
const CLARK_QA_EMAIL_DOMAIN = "clarkslabs.com";
const CREDENTIAL_NAMES = [
  "CLARK_QA_AUTH_NAME",
  "CLARK_QA_AUTH_EMAIL",
  "CLARK_QA_AUTH_PASSWORD",
];

function fingerprint(value) {
  return createHash("sha256").update(String(value)).digest("hex").slice(0, 16);
}

export function assertClarkOwnedQaEmail(email) {
  if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(email)) {
    throw new Error("CLARK_QA_AUTH_EMAIL is not a valid email address");
  }
  const domain = email.slice(email.lastIndexOf("@") + 1).toLowerCase();
  if (domain !== CLARK_QA_EMAIL_DOMAIN) {
    throw new Error(
      `CLARK_QA_AUTH_EMAIL must use the Clark-owned ${CLARK_QA_EMAIL_DOMAIN} domain`,
    );
  }
  return email;
}

export function loadQaAuthCredentials(envPath = DEFAULT_ENV_PATH) {
  const metadata = statSync(envPath);
  if ((metadata.mode & 0o077) !== 0) {
    throw new Error("the ignored .env must not be readable or writable by group/other users");
  }
  const fromFile = readIgnoredEnv(envPath, CREDENTIAL_NAMES);
  const name = process.env.CLARK_QA_AUTH_NAME || fromFile.CLARK_QA_AUTH_NAME;
  const email = process.env.CLARK_QA_AUTH_EMAIL || fromFile.CLARK_QA_AUTH_EMAIL;
  const password =
    process.env.CLARK_QA_AUTH_PASSWORD || fromFile.CLARK_QA_AUTH_PASSWORD;
  if (!name || !email || !password) {
    throw new Error(
      `Clark QA auth is missing; define ${CREDENTIAL_NAMES.join(", ")} in the ignored .env`,
    );
  }
  assertClarkOwnedQaEmail(email);
  if (password.length < 12) {
    throw new Error("CLARK_QA_AUTH_PASSWORD must contain at least 12 characters");
  }
  return {
    name,
    email,
    password,
    source: ".env",
    source_mode: metadata.mode & 0o777,
  };
}

export function parseJwtPayload(token) {
  if (typeof token !== "string") throw new Error("Clark JWT is missing");
  const segments = token.split(".");
  if (segments.length !== 3 || segments.some((segment) => !segment)) {
    throw new Error("Clark JWT is malformed");
  }
  try {
    return JSON.parse(Buffer.from(segments[1], "base64url").toString("utf8"));
  } catch {
    throw new Error("Clark JWT payload is malformed");
  }
}

export function cookieHeaderFromResponse(headers) {
  const values = typeof headers.getSetCookie === "function"
    ? headers.getSetCookie()
    : [headers.get("set-cookie")].filter(Boolean);
  const cookies = values
    .map((value) => String(value).split(";", 1)[0])
    .filter((value) => value.includes("="));
  if (!cookies.length) throw new Error("Clark sign-in returned no session cookie");
  return cookies.join("; ");
}

async function responseJson(response, label) {
  if (!response.ok) {
    throw new Error(`${label} failed with HTTP ${response.status}`);
  }
  try {
    return await response.json();
  } catch {
    throw new Error(`${label} returned invalid JSON`);
  }
}

export async function mintClarkQaSession({
  credentials = loadQaAuthCredentials(),
  authOrigin = DEFAULT_AUTH_ORIGIN,
  fetchImpl = globalThis.fetch,
  now = () => Date.now(),
} = {}) {
  if (typeof fetchImpl !== "function") throw new Error("fetch implementation is unavailable");
  assertClarkOwnedQaEmail(credentials.email);
  const origin = new URL(authOrigin).origin;
  const signedIn = await fetchImpl(`${origin}/api/auth/sign-in/email`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      origin,
    },
    body: JSON.stringify({
      email: credentials.email,
      password: credentials.password,
    }),
  });
  const cookie = cookieHeaderFromResponse(signedIn.headers);
  const signInBody = await responseJson(signedIn, "Clark QA email sign-in");
  const user = signInBody?.user;
  if (!user?.id || !user?.email) {
    throw new Error("Clark QA sign-in returned no stable user identity");
  }
  if (String(user.email).toLowerCase() !== credentials.email.toLowerCase()) {
    throw new Error("Clark QA sign-in returned a different account");
  }

  const minted = await fetchImpl(`${origin}/api/auth/token`, {
    headers: {
      cookie,
      origin,
    },
  });
  const tokenBody = await responseJson(minted, "Clark QA JWT mint");
  const token = tokenBody?.token;
  const claims = parseJwtPayload(token);
  const nowSeconds = Math.floor(now() / 1_000);
  if (claims.sub !== user.id) {
    throw new Error("Clark QA JWT subject does not match the signed-in account");
  }
  if (!Number.isInteger(claims.exp) || claims.exp <= nowSeconds) {
    throw new Error("Clark QA JWT is already expired");
  }
  if (claims.iss !== origin) {
    throw new Error("Clark QA JWT issuer does not match the auth origin");
  }

  return {
    retained_auth: {
      version: 2,
      descriptor: {
        user: {
          id: user.id,
          name: user.name || credentials.name,
          email: user.email,
          method: "local",
        },
      },
      authOrigin: origin,
      clarkToken: token,
      google: {
        accessToken: "",
        refreshToken: null,
        expiresAt: null,
      },
    },
    account: {
        id: user.id,
        name: user.name || credentials.name,
        email: user.email,
        method: "local",
    },
    account_fingerprint: fingerprint(user.id),
    issuer: claims.iss,
    expires_at: claims.exp,
    expires_in_seconds: claims.exp - nowSeconds,
    credential_recorded: false,
    required_user_vm_actions: 0,
  };
}

async function runCli() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    console.log(`Autonomous Clark QA authentication

Usage:
  node harness/clark-qa-auth.mjs probe

The probe signs in with the dedicated owner-only QA identity, mints a short-lived
Clark JWT, verifies its issuer, subject, and expiry, and prints only non-secret
metadata. It never prints the email, password, session cookie, or JWT.`);
    return;
  }
  if (args.length !== 1 || args[0] !== "probe") {
    throw new Error(`unknown arguments ${JSON.stringify(args)}`);
  }
  const result = await mintClarkQaSession();
  console.log(JSON.stringify({
    status: "passed",
    account_fingerprint: result.account_fingerprint,
    issuer: result.issuer,
    expires_in_seconds: result.expires_in_seconds,
    credential_recorded: false,
    required_user_vm_actions: 0,
  }));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await runCli();
}
