import { spawnSync } from "node:child_process";
import {
  chmodSync,
  statSync,
} from "node:fs";
import process from "node:process";

const SYSTEM_SID = "S-1-5-18";
let cachedWindowsSid;

function run(command, args) {
  const completed = spawnSync(command, args, {
    encoding: "utf8",
    timeout: 30_000,
    maxBuffer: 1024 * 1024,
  });
  if (completed.status !== 0) {
    throw new Error(
      `${command} failed: ${completed.stderr || completed.stdout || completed.error?.message}`,
    );
  }
  return completed.stdout || "";
}

function currentWindowsSid() {
  if (cachedWindowsSid) return cachedWindowsSid;
  const output = run("whoami.exe", ["/user", "/fo", "csv", "/nh"]);
  const match = output.match(/\bS-\d+(?:-\d+)+\b/);
  if (!match) throw new Error("whoami did not return a Windows security identifier");
  cachedWindowsSid = match[0];
  return cachedWindowsSid;
}

function windowsAccessSids(filePath) {
  const encodedPath = Buffer.from(String(filePath), "utf8").toString("base64");
  const script = [
    "$ErrorActionPreference = \"Stop\"",
    `$path = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String("${encodedPath}"))`,
    "$acl = Get-Acl -LiteralPath $path",
    "foreach ($entry in $acl.Access) {",
    "  $entry.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value",
    "}",
  ].join("; ");
  return run(
    "powershell.exe",
    [
      "-NoLogo",
      "-NoProfile",
      "-NonInteractive",
      "-ExecutionPolicy",
      "Bypass",
      "-Command",
      script,
    ],
  ).split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
}

export function secureOwnerOnlyFile(filePath) {
  chmodSync(filePath, 0o600);
  if (process.platform !== "win32") return;
  const currentSid = currentWindowsSid();
  const allowed = [...new Set([currentSid, SYSTEM_SID])];
  run("icacls.exe", [
    filePath,
    "/inheritance:r",
    "/grant:r",
    ...allowed.map((sid) => `*${sid}:(F)`),
  ]);
  const actual = windowsAccessSids(filePath);
  if (
    !actual.includes(currentSid)
    || actual.some((sid) => !allowed.includes(sid))
  ) {
    throw new Error("Windows file ACL is not restricted to its writer and SYSTEM");
  }
}

export function isOwnerOnlyFile(filePath) {
  if (process.platform !== "win32") {
    return (statSync(filePath).mode & 0o777) === 0o600;
  }
  const currentSid = currentWindowsSid();
  const allowed = new Set([currentSid, SYSTEM_SID]);
  const actual = windowsAccessSids(filePath);
  return actual.includes(currentSid) && actual.every((sid) => allowed.has(sid));
}
