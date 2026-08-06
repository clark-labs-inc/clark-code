#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { randomUUID } from "node:crypto";
import { createServer } from "node:http";
import { homedir, tmpdir } from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  bundleDataPath,
  ensureRemovableMediaSlot,
  qemuArgumentStrings,
  readUtmConfig,
  resetRemovableMedia,
  setQemuAdditionalArguments,
  setRemovableMediaSource,
} from "./utm-config.mjs";
import { parseGuestJson } from "./utm-guest-channel.mjs";
import { ejectInstallerMediaAndReset } from "./utm-install-media.mjs";
import { QmpClient } from "./utm-qmp.mjs";
import { parseUtmList, probeGuest } from "./utm-real-use.mjs";
import {
  buildUbuntuAutoinstall,
  buildWindowsOneShotAutologon,
  loadVmCredentials,
} from "./utm-unattended-config.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const inventoryPath = path.join(harnessDir, "clark-code-capability-inventory.json");
const envPath = path.join(repoDir, ".env");
const defaultUbuntuIso = path.join(
  homedir(),
  "Downloads",
  "UTM Installers",
  "ubuntu-24.04.4-desktop-arm64.iso",
);
const guestToolsIso = path.join(
  homedir(),
  "Library",
  "Containers",
  "com.utmapp.UTM",
  "Data",
  "Library",
  "Application Support",
  "GuestSupportTools",
  "utm-guest-tools-latest.iso",
);

function run(command, args, options = {}) {
  const completed = spawnSync(command, args, {
    encoding: "utf8",
    timeout: options.timeout_ms ?? 30_000,
    input: options.input,
    maxBuffer: 8 * 1024 * 1024,
  });
  return {
    ok: completed.status === 0,
    exit_code: completed.status,
    stdout: completed.stdout || "",
    stderr: completed.stderr || completed.error?.message || "",
  };
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function vmStatus(vmName) {
  const completed = run("utmctl", ["status", vmName], { timeout_ms: 10_000 });
  return completed.ok ? completed.stdout.trim() : "unknown";
}

async function stopVm(vmName, events) {
  const before = vmStatus(vmName);
  if (before === "stopped") return;
  run("utmctl", ["stop", vmName, "--request"], { timeout_ms: 15_000 });
  for (let attempt = 0; attempt < 30; attempt += 1) {
    await sleep(2_000);
    if (vmStatus(vmName) === "stopped") {
      events.push({ action: "stop", method: "guest_request", status: "passed" });
      return;
    }
  }
  const forced = run("utmctl", ["stop", vmName, "--force"], { timeout_ms: 20_000 });
  if (!forced.ok) throw new Error(forced.stderr || `cannot stop ${vmName}`);
  for (let attempt = 0; attempt < 15; attempt += 1) {
    await sleep(1_000);
    if (vmStatus(vmName) === "stopped") {
      events.push({ action: "stop", method: "vm_power_event", status: "passed" });
      return;
    }
  }
  throw new Error(`${vmName} did not stop`);
}

async function startVm(vmName, events) {
  if (vmStatus(vmName) === "started") return;
  const started = run("utmctl", ["start", vmName], { timeout_ms: 30_000 });
  if (!started.ok) throw new Error(started.stderr || `cannot start ${vmName}`);
  for (let attempt = 0; attempt < 30; attempt += 1) {
    await sleep(1_000);
    if (vmStatus(vmName) === "started") {
      events.push({ action: "start", status: "passed" });
      return;
    }
  }
  throw new Error(`${vmName} did not start`);
}

async function waitForProbe(platform, vmName, timeoutMs, label) {
  const deadline = Date.now() + timeoutMs;
  let last;
  let nextProgress = Date.now();
  while (Date.now() < deadline) {
    last = probeGuest(platform, vmName, vmStatus(vmName), run);
    if (last.ok) return last;
    if (Date.now() >= nextProgress) {
      console.log(`${label}: waiting for authenticated guest channel`);
      nextProgress = Date.now() + 30_000;
    }
    await sleep(3_000);
  }
  throw new Error(last?.error || `${label} guest channel timed out`);
}

function ensureQmpArguments(vmName, port, extraEntries = []) {
  const expected = [
    { value: "-qmp" },
    { value: `tcp:127.0.0.1:${port},server=on,wait=off` },
    ...extraEntries,
  ];
  const current = qemuArgumentStrings(readUtmConfig(vmName));
  if (
    current.length === expected.length
    && current.every((value, index) => value === expected[index].value)
  ) {
    return { changed: false, arguments: current };
  }
  const updated = setQemuAdditionalArguments(vmName, expected);
  return { changed: true, arguments: updated.arguments };
}

function findFirstFile(root, basename) {
  const entries = readdirSync(root, { recursive: true, withFileTypes: true });
  const found = entries.find((entry) => entry.isFile() && entry.name === basename);
  return found ? path.join(found.parentPath, found.name) : null;
}

function extractWindowsGuestAgent() {
  if (!existsSync(guestToolsIso)) throw new Error("official UTM guest-tools ISO is missing");
  const temporary = mkdtempSync(path.join(tmpdir(), "clark-utm-qga-"));
  const outer = path.join(temporary, "outer");
  const inner = path.join(temporary, "inner");
  mkdirSync(outer);
  mkdirSync(inner);
  const first = run("7z", ["x", "-y", `-o${outer}`, guestToolsIso]);
  if (!first.ok) throw new Error(first.stderr || "cannot extract UTM guest tools");
  const installer = findFirstFile(outer, "utm-guest-tools-0.1.271.exe")
    || readdirSync(outer, { recursive: true, withFileTypes: true })
      .filter((entry) => entry.isFile() && /^utm-guest-tools-.*\.exe$/i.test(entry.name))
      .map((entry) => path.join(entry.parentPath, entry.name))[0];
  if (!installer) throw new Error("UTM guest-tools installer is absent from its ISO");
  const second = run("7z", ["x", "-y", `-o${inner}`, installer]);
  if (!second.ok) throw new Error(second.stderr || "cannot inspect UTM guest-tools installer");
  const msi = findFirstFile(inner, "qemu-ga-x86_64.msi");
  if (!msi) throw new Error("QEMU guest-agent MSI is absent from official UTM guest tools");
  return { temporary, msi };
}

async function bindArtifactServer(host, routes) {
  const requests = [];
  const server = createServer((request, response) => {
    const body = routes[request.url];
    requests.push({
      path: request.url,
      method: request.method,
      remote_address: request.socket.remoteAddress,
      at: new Date().toISOString(),
      served: body !== undefined,
    });
    if (body === undefined) {
      response.writeHead(404);
      response.end("not found\n");
      return;
    }
    response.writeHead(200, {
      "Content-Type": request.url.endsWith(".msi")
        ? "application/octet-stream"
        : "text/plain; charset=utf-8",
      "Content-Length": Buffer.byteLength(body),
      "Cache-Control": "no-store",
    });
    response.end(body);
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, host, resolve);
  });
  const address = server.address();
  return {
    port: address.port,
    requests,
    close: () => new Promise((resolve) => server.close(resolve)),
  };
}

async function bootstrapWindowsGuestAgent(environment, credentials, hostIp, events) {
  const extracted = extractWindowsGuestAgent();
  try {
    const msi = readFileSync(extracted.msi);
    const server = await bindArtifactServer(hostIp, { "/qemu-ga.msi": msi });
    try {
      const command = `cmd /c curl.exe -f http://${hostIp}:${server.port}/qemu-ga.msi -o c:\\users\\public\\qga.msi && msiexec /i c:\\users\\public\\qga.msi /qn /norestart`;
      const client = new QmpClient({ port: environment.autonomy.qmp_port, timeoutMs: 10_000 });
      await client.connect();
      await client.openWindowsRunAndExecute(command);
      client.close();
      try {
        const probe = await waitForProbe("windows", environment.vm_name, 120_000, "Windows bootstrap");
        events.push({ action: "guest_agent_bootstrap", status: "passed", login_recovery: false });
        return probe;
      } catch {
        const recovery = new QmpClient({ port: environment.autonomy.qmp_port, timeoutMs: 10_000 });
        await recovery.connect();
        await recovery.sendChord(["esc"]);
        await recovery.sendChord(["ctrl", "a"]);
        await recovery.sendChord(["backspace"]);
        await recovery.typeText(credentials.password);
        await recovery.sendChord(["ret"]);
        await sleep(15_000);
        await recovery.openWindowsRunAndExecute(command);
        recovery.close();
        const probe = await waitForProbe("windows", environment.vm_name, 180_000, "Windows recovery");
        events.push({ action: "guest_agent_bootstrap", status: "passed", login_recovery: true });
        return probe;
      }
    } finally {
      await server.close();
    }
  } finally {
    rmSync(extracted.temporary, { recursive: true, force: true });
  }
}

async function armWindowsAutologon(environment, credentials) {
  const nonce = randomUUID().replaceAll("-", "");
  const scriptPath = `C:\\Users\\Public\\clark-qa-arm-${nonce}.ps1`;
  const markerPath = `C:\\Users\\Public\\clark-qa-arm-${nonce}.json`;
  const script = `${buildWindowsOneShotAutologon(credentials)}
[ordered]@{ status = "armed"; probe_marker = "${nonce}" } | ConvertTo-Json -Compress | Set-Content -LiteralPath "${markerPath}" -Encoding UTF8
`;
  run("utmctl", ["file", "push", environment.vm_name, scriptPath], {
    input: script,
    timeout_ms: 30_000,
  });
  run("utmctl", [
    "exec",
    environment.vm_name,
    "--cmd",
    "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
    "-NoProfile",
    "-NonInteractive",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    scriptPath,
  ], { timeout_ms: 30_000 });
  let armed = false;
  for (let attempt = 0; attempt < 60 && !armed; attempt += 1) {
    await sleep(250);
    const pulled = run("utmctl", ["file", "pull", environment.vm_name, markerPath]);
    armed = parseGuestJson(pulled.stdout, nonce)?.status === "armed";
  }
  run("utmctl", [
    "exec",
    environment.vm_name,
    "--cmd",
    "C:\\Windows\\System32\\cmd.exe",
    "/c",
    "del",
    "/f",
    scriptPath,
    markerPath,
  ]);
  if (!armed) throw new Error("Windows one-shot autologon did not arm");
}

async function ensureWindows(environment, credentials, hostIp) {
  const events = [];
  let probe = probeGuest("windows", environment.vm_name, vmStatus(environment.vm_name), run);
  if (!probe.ok) {
    await stopVm(environment.vm_name, events);
    ensureQmpArguments(environment.vm_name, environment.autonomy.qmp_port);
    await startVm(environment.vm_name, events);
    probe = await bootstrapWindowsGuestAgent(environment, credentials, hostIp, events);
  }
  await armWindowsAutologon(environment, credentials);
  events.push({ action: "one_shot_autologon_arm", status: "passed" });
  await stopVm(environment.vm_name, events);
  ensureQmpArguments(environment.vm_name, environment.autonomy.qmp_port);
  await startVm(environment.vm_name, events);
  const after = await waitForProbe("windows", environment.vm_name, 300_000, "Windows reboot");
  for (let attempt = 0; attempt < 20 && after.data.autologon_password_present; attempt += 1) {
    await sleep(2_000);
    probe = probeGuest("windows", environment.vm_name, vmStatus(environment.vm_name), run);
    if (probe.ok) Object.assign(after, probe);
  }
  const usernameReady = String(after.data.interactive_user || "").toLowerCase()
    .endsWith(`\\${credentials.username.toLowerCase()}`);
  if (!usernameReady || !after.data.desktop_shell_running) {
    throw new Error("Windows did not reach the configured autonomous desktop session");
  }
  if (after.data.autologon_password_present) {
    throw new Error("Windows autologon cleanup left a password in Winlogon");
  }
  events.push({ action: "reboot_to_gui_without_user", status: "passed" });
  return { status: "passed", required_user_vm_actions: 0, probe: after.data, events };
}

function passwordHash(password) {
  const hashed = run("/usr/sbin/htpasswd", ["-niB", "-C", "12", "clarkqa"], {
    input: `${password}\n`,
    timeout_ms: 10_000,
  });
  const value = hashed.stdout.trim().split(":", 2)[1] || "";
  if (!hashed.ok || !value.startsWith("$2y$")) {
    throw new Error("cannot create Ubuntu bcrypt password hash");
  }
  return value;
}

function buildUbuntuSeedIso(vmName, userData) {
  const temporary = mkdtempSync(path.join(tmpdir(), "clark-ubuntu-cidata-"));
  const seedRoot = path.join(repoDir, "target", "utm-seeds");
  mkdirSync(seedRoot, { recursive: true, mode: 0o700 });
  chmodSync(seedRoot, 0o700);
  const output = path.join(
    seedRoot,
    `${vmName.replaceAll(/[^A-Za-z0-9]+/g, "-")}-${randomUUID()}.iso`,
  );
  try {
    writeFileSync(path.join(temporary, "user-data"), userData, { mode: 0o600 });
    writeFileSync(
      path.join(temporary, "meta-data"),
      "instance-id: clark-qa-ubuntu\nlocal-hostname: clark-qa-ubuntu\n",
      { mode: 0o600 },
    );
    writeFileSync(path.join(temporary, "vendor-data"), "#cloud-config\n", { mode: 0o600 });
    const built = run("hdiutil", [
      "makehybrid",
      "-quiet",
      "-o",
      output,
      "-iso",
      "-joliet",
      "-default-volume-name",
      "CIDATA",
      temporary,
    ], { timeout_ms: 120_000 });
    if (!built.ok || !existsSync(output)) {
      throw new Error(built.stderr || "cannot create the Ubuntu NoCloud seed ISO");
    }
    chmodSync(output, 0o600);
    return output;
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}

async function sendAutoinstallConfirmation(port) {
  const client = new QmpClient({ port, timeoutMs: 10_000 });
  try {
    await client.connect();
    await client.typeText("yes");
    await client.sendChord(["ret"]);
    await sleep(1_000);
    await client.sendChord(["tab"]);
    await client.sendChord(["ret"]);
    return true;
  } catch {
    return false;
  } finally {
    client.close();
  }
}

export function shouldSendAutoinstallConfirmation(confirmations, now, dueAt) {
  return confirmations === 0 && now >= dueAt;
}

function ubuntuDiskAllocatedBytes(vmName) {
  const config = readUtmConfig(vmName);
  const disk = (config.Drive || []).find(
    (drive) => drive.ImageType === "Disk" && !drive.ReadOnly && drive.ImageName,
  );
  if (!disk) return 0;
  const metadata = statSync(bundleDataPath(vmName, path.basename(disk.ImageName)), {
    bigint: true,
  });
  return Number(metadata.blocks * 512n);
}

async function remediateUbuntuInitialSetup(environment, credentials, events) {
  const username = JSON.stringify(credentials.username);
  const basename = `clark-qa-initial-setup-${randomUUID().replaceAll("-", "")}.py`;
  const scriptPath = `/var/tmp/${basename}`;
  const script = `import os, pathlib, pwd, subprocess
username = ${username}
account = pwd.getpwnam(username)
config = pathlib.Path(account.pw_dir) / ".config"
config.mkdir(mode=0o700, parents=True, exist_ok=True)
os.chown(config, account.pw_uid, account.pw_gid)
marker = config / "gnome-initial-setup-done"
marker.touch(mode=0o600, exist_ok=True)
os.chown(marker, account.pw_uid, account.pw_gid)
subprocess.run(
    ["/usr/bin/pkill", "-u", username, "-f", "^/usr/libexec/gnome-initial-setup"],
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
    timeout=10,
)
`;
  run("utmctl", ["file", "push", environment.vm_name, scriptPath], {
    input: script,
    timeout_ms: 30_000,
  });
  run("utmctl", [
    "exec",
    environment.vm_name,
    "--cmd",
    "/usr/bin/python3",
    scriptPath,
  ], { timeout_ms: 30_000 });
  let verified;
  for (let attempt = 0; attempt < 10; attempt += 1) {
    await sleep(1_000);
    verified = probeGuest(
      "ubuntu",
      environment.vm_name,
      vmStatus(environment.vm_name),
      run,
    );
    if (
      verified.ok
      && verified.data.initial_setup_done === true
      && verified.data.initial_setup_running === false
    ) {
      break;
    }
  }
  run("utmctl", [
    "exec",
    environment.vm_name,
    "--cmd",
    "/usr/bin/rm",
    "-f",
    scriptPath,
  ]);
  if (
    !verified?.ok
    || verified.data.initial_setup_done !== true
    || verified.data.initial_setup_running !== false
  ) {
    throw new Error(verified?.error || "Ubuntu initial-setup suppression did not verify");
  }
  events.push({
    action: "initial_setup_suppression",
    status: "passed",
    authenticated_guest_channel: true,
  });
}

async function provisionUbuntu(environment, credentials, hostIp) {
  const events = [];
  const isoPath = process.env.CLARK_QA_UBUNTU_DESKTOP_ISO || defaultUbuntuIso;
  if (!existsSync(isoPath) || !/desktop.*arm64\.iso$/i.test(path.basename(isoPath))) {
    throw new Error("the Ubuntu Desktop ARM64 ISO is missing or is not a Desktop image");
  }
  await stopVm(environment.vm_name, events);
  setRemovableMediaSource(environment.vm_name, 1, isoPath);
  const userData = buildUbuntuAutoinstall({
    username: credentials.username,
    passwordHash: passwordHash(credentials.password),
  });
  const seedIso = buildUbuntuSeedIso(environment.vm_name, userData);
  ensureRemovableMediaSlot(environment.vm_name, 2, seedIso);
  let completed = false;
  let mediaSourcesCleared = false;
  let confirmations = 0;
  try {
    ensureQmpArguments(
      environment.vm_name,
      environment.autonomy.qmp_port,
      [{ value: "-no-reboot" }],
    );
    await startVm(environment.vm_name, events);
    const deadline = Date.now() + 60 * 60 * 1_000;
    let nextConfirmation = Date.now() + 60_000;
    let nextProgress = 0;
    while (vmStatus(environment.vm_name) !== "stopped" && Date.now() < deadline) {
      const allocatedBytes = ubuntuDiskAllocatedBytes(environment.vm_name);
      if (Date.now() >= nextProgress) {
        console.log(
          `Ubuntu unattended install: disk_mib=${Math.floor(allocatedBytes / 1024 / 1024)}; automated_confirmations=${confirmations}`,
        );
        nextProgress = Date.now() + 30_000;
      }
      if (shouldSendAutoinstallConfirmation(confirmations, Date.now(), nextConfirmation)) {
        if (await sendAutoinstallConfirmation(environment.autonomy.qmp_port)) {
          confirmations += 1;
        }
        nextConfirmation = Date.now() + 30_000;
      }
      await sleep(5_000);
    }
    if (vmStatus(environment.vm_name) !== "stopped") {
      events.push({
        action: "installer_poweroff",
        status: "recovery_required",
        reason: "installer did not honor shutdown: poweroff before the bounded deadline",
      });
    }
    if (vmStatus(environment.vm_name) === "stopped") {
      ensureQmpArguments(environment.vm_name, environment.autonomy.qmp_port);
      await startVm(environment.vm_name, events);
    }
    const media = [path.basename(isoPath), path.basename(seedIso)];
    const firstBootRecovery = await ejectInstallerMediaAndReset({
      port: environment.autonomy.qmp_port,
      expectedBasenames: media,
    });
    events.push({
      action: "installer_media_eject",
      status: "passed",
      method: firstBootRecovery.transport,
      media: firstBootRecovery.ejected,
    });
    const firstBoot = await waitForProbe(
      "ubuntu",
      environment.vm_name,
      10 * 60_000,
      "Ubuntu first boot",
    );
    if (
      firstBoot.data.live_session
      || firstBoot.data.os_id !== "ubuntu"
      || !firstBoot.data.ubuntu_desktop_installed
      || firstBoot.data.qemu_ga_service !== "active"
    ) {
      throw new Error("Ubuntu first boot is not the installed integrated Desktop environment");
    }
    events.push({
      action: "ubuntu_desktop_autoinstall",
      status: "passed",
      seed_transport: "nocloud_cidata_iso",
      automated_confirmations: confirmations,
      installed_disk_verified: true,
    });
    events.push({ action: "first_gui_boot_without_user", status: "passed" });
    await remediateUbuntuInitialSetup(environment, credentials, events);
    await stopVm(environment.vm_name, events);
    resetRemovableMedia(environment.vm_name);
    mediaSourcesCleared = true;
    ensureQmpArguments(environment.vm_name, environment.autonomy.qmp_port);
    await startVm(environment.vm_name, events);
    const probe = await waitForProbe(
      "ubuntu",
      environment.vm_name,
      10 * 60_000,
      "Ubuntu clean reboot",
    );
    if (
      probe.data.live_session
      || probe.data.os_id !== "ubuntu"
      || !probe.data.ubuntu_desktop_installed
      || probe.data.qemu_ga_service !== "active"
      || probe.data.initial_setup_done !== true
      || probe.data.initial_setup_running !== false
    ) {
      throw new Error("Ubuntu clean reboot is not the installed integrated Desktop environment");
    }
    events.push({ action: "clean_gui_reboot_without_user", status: "passed" });
    completed = true;
    return { status: "passed", required_user_vm_actions: 0, probe: probe.data, events };
  } finally {
    if (!completed && vmStatus(environment.vm_name) === "started") {
      await stopVm(environment.vm_name, events);
    }
    if (mediaSourcesCleared && existsSync(seedIso)) rmSync(seedIso);
  }
}

async function ensureUbuntu(environment, credentials, hostIp) {
  const existing = probeGuest("ubuntu", environment.vm_name, vmStatus(environment.vm_name), run);
  if (
    existing.ok
    && existing.data.live_session === false
    && existing.data.ubuntu_desktop_installed
    && existing.data.qemu_ga_service === "active"
  ) {
    const events = [{ action: "reuse_installed_desktop", status: "passed" }];
    await remediateUbuntuInitialSetup(environment, credentials, events);
    await stopVm(environment.vm_name, events);
    ensureQmpArguments(environment.vm_name, environment.autonomy.qmp_port);
    await startVm(environment.vm_name, events);
    const after = await waitForProbe(
      "ubuntu",
      environment.vm_name,
      10 * 60_000,
      "Ubuntu autonomous reboot",
    );
    if (
      after.data.live_session
      || after.data.os_id !== "ubuntu"
      || !after.data.ubuntu_desktop_installed
      || after.data.qemu_ga_service !== "active"
      || after.data.initial_setup_done !== true
      || after.data.initial_setup_running !== false
    ) {
      throw new Error("Ubuntu did not return to the installed integrated Desktop environment");
    }
    events.push({ action: "clean_gui_reboot_without_user", status: "passed" });
    return {
      status: "passed",
      required_user_vm_actions: 0,
      probe: after.data,
      events,
    };
  }
  return provisionUbuntu(environment, credentials, hostIp);
}

function hostSharedAddress() {
  const found = run("ipconfig", ["getifaddr", "bridge100"]);
  if (found.ok && /^192\.168\.\d+\.\d+$/.test(found.stdout.trim())) {
    return found.stdout.trim();
  }
  const configured = run("ifconfig", ["bridge100"]);
  const match = configured.stdout.match(/\binet (192\.168\.\d+\.\d+)\b/);
  if (!configured.ok || !match) throw new Error("cannot resolve the UTM shared-network host address");
  return match[1];
}

function valueArg(args, name) {
  const inline = args.find((arg) => arg.startsWith(`${name}=`));
  if (inline) return inline.slice(name.length + 1);
  const index = args.indexOf(name);
  if (index < 0) return undefined;
  const value = args[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

function safeError(error, password) {
  return String(error?.message || error || "unknown error")
    .replaceAll(password, "[REDACTED]")
    .replace(/\b(?:sk-|ck_(?:live|test)_)[A-Za-z0-9._-]+\b/g, "[REDACTED]");
}

async function runCli() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    console.log(`Clark Code autonomous UTM lifecycle

Usage:
  node harness/utm-autonomy.mjs audit [--out NEW_DIRECTORY]
  node harness/utm-autonomy.mjs ensure [--out NEW_DIRECTORY]
    [--platform all|windows|ubuntu]

ensure provisions, logs in, bootstraps guest transport, reboots, and verifies
the exact UTM guests without physical input. Ubuntu uses unattended Desktop
autoinstall. Windows uses one-shot autologon with automatic secret cleanup.
No Parallels integration exists.`);
    return;
  }
  const action = args[0];
  if (!["audit", "ensure"].includes(action)) throw new Error("action must be audit or ensure");
  const outputArg = valueArg(args, "--out")
    || path.join(
      "target",
      "utm-autonomy",
      `${new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z")}-${process.pid}`,
    );
  const selected = valueArg(args, "--platform") || "all";
  const platforms = selected === "all" ? ["windows", "ubuntu"] : [selected];
  if (platforms.some((platform) => !["windows", "ubuntu"].includes(platform))) {
    throw new Error("--platform must be all, windows, or ubuntu");
  }
  const outputDir = path.resolve(repoDir, outputArg);
  if (existsSync(outputDir)) throw new Error(`refusing to overwrite ${outputDir}`);
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);
  const inventory = JSON.parse(readFileSync(inventoryPath, "utf8"));
  const credentials = loadVmCredentials(envPath);
  const listed = run("utmctl", ["list"]);
  const registrations = listed.ok ? parseUtmList(listed.stdout) : [];
  const receipt = {
    schema_version: 1,
    benchmark: "clark_code_utm_full_autonomy",
    action,
    status: "running",
    generated_at: new Date().toISOString(),
    virtualization: "utm",
    forbidden_virtualization_invocations: 0,
    required_user_vm_actions: 0,
    credential_source: {
      path: credentials.source,
      keys: ["CLARK_QA_VM_USERNAME", "CLARK_QA_VM_PASSWORD"],
      mode: credentials.source_mode.toString(8).padStart(3, "0"),
      values_persisted: false,
    },
    guests: {},
  };
  const receiptPath = path.join(outputDir, "receipt.json");
  const persist = () => writeFileSync(
    receiptPath,
    `${JSON.stringify(receipt, null, 2)}\n`,
    { mode: 0o600 },
  );
  persist();
  const hostIp = action === "ensure" ? hostSharedAddress() : null;
  for (const platform of platforms) {
    const environment = inventory.real_use_environments[platform];
    const registration = registrations.find((guest) => guest.name === environment.vm_name);
    if (!registration) {
      receipt.guests[platform] = { status: "failed", error: "exact UTM VM is not registered" };
      continue;
    }
    if (action === "audit") {
      const qemuArguments = qemuArgumentStrings(readUtmConfig(environment.vm_name));
      const qmpConfigured = qemuArguments[0] === "-qmp"
        && qemuArguments[1]
          === `tcp:127.0.0.1:${environment.autonomy.qmp_port},server=on,wait=off`;
      receipt.guests[platform] = {
        status: qmpConfigured ? "passed" : "blocked",
        registered: true,
        vm_name: environment.vm_name,
        qmp_port: environment.autonomy.qmp_port,
        qmp_configured: qmpConfigured,
        qemu_arguments: qemuArguments,
        required_user_vm_actions: 0,
      };
      continue;
    }
    try {
      receipt.guests[platform] = platform === "windows"
        ? await ensureWindows(environment, credentials, hostIp)
        : await ensureUbuntu(environment, credentials, hostIp);
    } catch (error) {
      receipt.guests[platform] = {
        status: "failed",
        required_user_vm_actions: 0,
        error: safeError(error, credentials.password),
      };
    }
    persist();
  }
  receipt.status = Object.values(receipt.guests).every((guest) => guest.status === "passed")
    ? "passed"
    : "failed";
  receipt.completed_at = new Date().toISOString();
  persist();
  console.log(JSON.stringify({
    status: receipt.status,
    required_user_vm_actions: 0,
    guests: Object.fromEntries(
      Object.entries(receipt.guests).map(([platform, guest]) => [platform, guest.status]),
    ),
  }));
  console.log(`RECEIPT=${receiptPath}`);
  if (receipt.status !== "passed") process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await runCli();
}
