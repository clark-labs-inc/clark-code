#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  accessSync,
  chmodSync,
  mkdirSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { executeGuestJson } from "./utm-guest-channel.mjs";
import { captureUtmWindowObservation } from "./utm-window-observation.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const inventoryPath = path.join(harnessDir, "clark-code-capability-inventory.json");
const featureMapPath = path.join(harnessDir, "clark-code-feature-map.json");
const DEFAULT_UTM_ROOT = path.join(
  homedir(),
  "Library",
  "Containers",
  "com.utmapp.UTM",
  "Data",
  "Documents",
);
const PLATFORM_THRESHOLDS = {
  windows: 5 * 1024 * 1024 * 1024,
  ubuntu: 2 * 1024 * 1024 * 1024,
};

export function parseUtmList(source) {
  const guests = [];
  for (const line of String(source).split(/\r?\n/).slice(1)) {
    const match = line.match(
      /^([0-9A-Fa-f]{8}(?:-[0-9A-Fa-f]{4}){3}-[0-9A-Fa-f]{12})\s+(\S+)\s+(.+?)\s*$/,
    );
    if (match) guests.push({ uuid: match[1].toUpperCase(), status: match[2], name: match[3] });
  }
  return guests;
}

export function parseJsonTail(source) {
  const lines = String(source).trim().split(/\r?\n/).reverse();
  for (const line of lines) {
    try {
      const value = JSON.parse(line);
      if (value && typeof value === "object" && !Array.isArray(value)) return value;
    } catch {
      // UTM may print non-JSON diagnostics before the guest command's final line.
    }
  }
  return null;
}

export function redact(value) {
  return String(value)
    .replace(/\bck_(?:live|test)_[A-Za-z0-9._-]+\b/g, "ck_[REDACTED]")
    .replace(/\bsk-[A-Za-z0-9_-]{16,}\b/g, "sk-[REDACTED]")
    .replace(/(authorization\s*[:=]\s*bearer\s+)\S+/gi, "$1[REDACTED]")
    .slice(-4_000);
}

function result(command, args, options = {}) {
  const completed = spawnSync(command, args, {
    encoding: "utf8",
    timeout: options.timeout_ms ?? 15_000,
    maxBuffer: 4 * 1024 * 1024,
    input: options.input,
  });
  return {
    ok: completed.status === 0,
    exit_code: completed.status,
    signal: completed.signal,
    stdout: redact(completed.stdout || ""),
    stderr: redact(completed.stderr || completed.error?.message || ""),
  };
}

function safeVmName(name) {
  if (
    typeof name !== "string"
    || !name.trim()
    || name.includes("/")
    || name.includes("\\")
    || name === "."
    || name === ".."
  ) {
    throw new Error(`unsafe UTM VM name ${JSON.stringify(name)}`);
  }
  return name;
}

function inspectConfiguration(vmName, utmRoot = DEFAULT_UTM_ROOT) {
  safeVmName(vmName);
  const bundle = path.join(utmRoot, `${vmName}.utm`);
  const configPath = path.join(bundle, "config.plist");
  const converted = result("plutil", ["-convert", "json", "-o", "-", configPath]);
  if (!converted.ok) {
    return {
      ok: false,
      bundle: path.basename(bundle),
      error: converted.stderr || "cannot read UTM configuration",
      config: null,
      disks: [],
    };
  }
  const config = JSON.parse(converted.stdout);
  const dataDir = path.join(bundle, "Data");
  const disks = (config.Drive || [])
    .filter((drive) => drive.ImageType === "Disk" && !drive.ReadOnly && drive.ImageName)
    .map((drive) => {
      const diskPath = path.join(dataDir, path.basename(drive.ImageName));
      try {
        const metadata = statSync(diskPath, { bigint: true });
        return {
          name: path.basename(diskPath),
          file_size_bytes: Number(metadata.size),
          allocated_bytes: Number(metadata.blocks * 512n),
        };
      } catch (error) {
        return { name: path.basename(diskPath), error: error.message };
      }
    });
  return {
    ok: true,
    bundle: path.basename(bundle),
    config,
    disks,
  };
}

const ubuntuProbe = String.raw`
import json, pathlib, shutil, subprocess, sys
def run(*args):
    try:
        return subprocess.run(args, text=True, capture_output=True, timeout=5).stdout.strip()
    except Exception:
        return ""
release = {}
for line in pathlib.Path("/etc/os-release").read_text(errors="replace").splitlines():
    if "=" in line:
        key, value = line.split("=", 1)
        release[key] = value.strip().strip('"')
cmdline = pathlib.Path("/proc/cmdline").read_text(errors="replace")
root_source = run("findmnt", "-n", "-o", "SOURCE", "/")
payload = {
    "os_id": release.get("ID"),
    "os_version": release.get("VERSION_ID"),
    "architecture": run("uname", "-m"),
    "live_session": pathlib.Path("/rofs").exists() or "boot=casper" in cmdline or root_source == "overlay",
    "root_source": root_source,
    "desktop_target": run("systemctl", "get-default"),
    "display_manager_active": run("systemctl", "is-active", "display-manager") == "active",
    "ubuntu_desktop_installed": run("dpkg-query", "-W", "-f=" + "$" + "{Status}", "ubuntu-desktop").endswith("ok installed"),
    "spice_agent_installed": shutil.which("spice-vdagent") is not None,
    "bubblewrap_installed": shutil.which("bwrap") is not None,
    "qemu_ga_service": run("systemctl", "is-active", "qemu-guest-agent"),
    "initial_setup_running": bool(run(
        "pgrep",
        "-f",
        "^/usr/libexec/gnome-initial-setup",
    )),
    "initial_setup_done": any(
        pathlib.Path("/home").glob("*/.config/gnome-initial-setup-done")
    ),
    "graphical_session_count": sum(
        1 for line in run("loginctl", "list-sessions", "--no-legend").splitlines()
        if line.strip()
    ),
    "clark_code_installed": any(pathlib.Path(item).exists() for item in [
        "/usr/bin/clark-code",
        "/usr/local/bin/clark-code",
        "/opt/Clark Code/clark-code",
    ]),
}
`;

export const windowsProbe = String.raw`
$ErrorActionPreference = "Stop"
$uninstall = @(
  "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*",
  "HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*"
) | ForEach-Object { Get-ItemProperty $_ -ErrorAction SilentlyContinue } |
  Where-Object { $_.DisplayName -like "*Clark Code*" }
$userUninstall = Get-ChildItem "Registry::HKEY_USERS" -ErrorAction SilentlyContinue |
  ForEach-Object {
    Get-ItemProperty (
      "Registry::" + $_.Name + "\Software\Microsoft\Windows\CurrentVersion\Uninstall\*"
    ) -ErrorAction SilentlyContinue
  } | Where-Object { $_.DisplayName -like "*Clark Code*" }
$clarkExecutables = @(
  @(
    Get-ChildItem "C:\Users\*\AppData\Local\Clark Code\clark-desktop.exe" -File -ErrorAction SilentlyContinue
    Get-ChildItem "C:\Program Files\Clark Code\clark-desktop.exe" -File -ErrorAction SilentlyContinue
  ) | Select-Object -ExpandProperty FullName -Unique
)
$clarkProcesses = @(Get-Process -ErrorAction SilentlyContinue |
  Where-Object { $_.ProcessName -eq "clark-desktop" })
$computerSystem = Get-CimInstance Win32_ComputerSystem
$winlogon = Get-ItemProperty "HKLM:\Software\Microsoft\Windows NT\CurrentVersion\Winlogon"
$payload = [ordered]@{
  os_caption = (Get-CimInstance Win32_OperatingSystem).Caption
  os_version = (Get-CimInstance Win32_OperatingSystem).Version
  architecture = $computerSystem.SystemType
  desktop_shell_running = [bool](Get-Process explorer -ErrorAction SilentlyContinue)
  interactive_user = $computerSystem.UserName
  qemu_ga_service = (Get-Service qemu-ga -ErrorAction SilentlyContinue).Status.ToString()
  autologon_enabled = $winlogon.AutoAdminLogon -eq "1"
  autologon_password_present = [bool]$winlogon.PSObject.Properties["DefaultPassword"]
  uac_enable_lua = (
    Get-ItemPropertyValue "HKLM:\Software\Microsoft\Windows\CurrentVersion\Policies\System" -Name EnableLUA -ErrorAction SilentlyContinue
  )
  clark_code_installed = [bool]($uninstall -or $userUninstall -or $clarkExecutables)
  clark_code_executables = @($clarkExecutables)
  clark_code_running = [bool]$clarkProcesses
  clark_code_process_count = $clarkProcesses.Count
  clark_code_file_version = if ($clarkExecutables) {
    (Get-Item $clarkExecutables[0]).VersionInfo.FileVersion
  } else {
    $null
  }
}
`;

export function probeGuest(platform, vmName, state, run = result) {
  return executeGuestJson({
    platform,
    vmName,
    state,
    probeSource: platform === "ubuntu" ? ubuntuProbe : windowsProbe,
    run,
  });
}

function check(id, passed, evidence) {
  return { id, status: passed ? "passed" : "failed", evidence };
}

export function evaluateGuest({
  platform,
  environment,
  registration,
  configuration,
  probe,
  observation,
}) {
  const config = configuration?.config || {};
  const diskBytes = (configuration?.disks || []).reduce(
    (total, disk) => total + (disk.allocated_bytes || 0),
    0,
  );
  const checks = [
    check("registered_exact_vm", Boolean(registration), registration?.uuid || "not registered"),
    check("vm_started", registration?.status === "started", registration?.status || "unknown"),
    check(
      "configuration_identity",
      configuration?.ok && config.Information?.Name === environment.vm_name,
      configuration?.bundle || configuration?.error,
    ),
    check("graphical_display_configured", (config.Display || []).length > 0, config.Display?.[0]?.Hardware),
    check(
      "writable_system_disk_configured",
      (configuration?.disks || []).length > 0,
      configuration?.disks || [],
    ),
    check(
      "installed_disk_footprint",
      diskBytes >= PLATFORM_THRESHOLDS[platform],
      { allocated_bytes: diskBytes, minimum_bytes: PLATFORM_THRESHOLDS[platform] },
    ),
    check("shared_network_configured", config.Network?.[0]?.Mode === "Shared", config.Network?.[0]?.Mode),
    check("guest_agent_command_channel", probe?.ok === true, probe?.error || "probe JSON received"),
    check(
      "fresh_gui_observation",
      observation?.gui_visible === true,
      observation?.finding || "no verified GUI observation supplied",
    ),
  ];
  if (platform === "ubuntu" && probe?.data) {
    checks.push(
      check("ubuntu_os", probe.data.os_id === "ubuntu", probe.data.os_id),
      check("installed_not_live_media", probe.data.live_session === false, probe.data.root_source),
      check("graphical_boot_target", probe.data.desktop_target === "graphical.target", probe.data.desktop_target),
      check("display_manager_active", probe.data.display_manager_active === true, probe.data.display_manager_active),
      check("ubuntu_desktop_installed", probe.data.ubuntu_desktop_installed === true, probe.data.ubuntu_desktop_installed),
      check("spice_guest_integration", probe.data.spice_agent_installed === true, probe.data.spice_agent_installed),
      check("bubblewrap_available", probe.data.bubblewrap_installed === true, probe.data.bubblewrap_installed),
      check("qemu_guest_agent_active", probe.data.qemu_ga_service === "active", probe.data.qemu_ga_service),
      check(
        "initial_setup_complete",
        probe.data.initial_setup_done === true && probe.data.initial_setup_running === false,
        {
          done_marker: probe.data.initial_setup_done,
          process_running: probe.data.initial_setup_running,
        },
      ),
    );
  }
  if (platform === "windows" && probe?.data) {
    checks.push(
      check("windows_11_os", /Windows 11/i.test(probe.data.os_caption || ""), probe.data.os_caption),
      check("windows_desktop_shell", probe.data.desktop_shell_running === true, probe.data.desktop_shell_running),
      check("qemu_guest_agent_active", probe.data.qemu_ga_service === "Running", probe.data.qemu_ga_service),
      check("tpm_configured", config.QEMU?.TPMDevice === true, config.QEMU?.TPMDevice),
    );
  }
  const environmentReady = checks.every((item) => item.status === "passed");
  return {
    platform,
    vm_name: environment.vm_name,
    status: environmentReady ? "ready" : "blocked",
    environment_ready: environmentReady,
    product_installed: probe?.data?.clark_code_installed === true,
    checks,
    probe: {
      status: probe?.ok ? "passed" : "failed",
      data: probe?.data || null,
      error: probe?.error || null,
    },
  };
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

async function runCli() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    console.log(`Clark Code UTM real-use environment preflight

Usage:
  node harness/utm-real-use.mjs [--platform all|windows|ubuntu] [--out PATH]
    [--observation-receipt PATH] [--allow-blocked]

This command is read-only apart from waking a display with localhost-only QMP.
It autonomously captures the exact UTM windows, verifies the checked-in guest
contract, and writes an owner-only receipt. Real product scenarios remain
not_run until this preflight is ready.`);
    return;
  }
  const knownFlags = new Set(["--allow-blocked", "--help", "-h"]);
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (knownFlags.has(arg)) continue;
    if (["--platform", "--out", "--observation-receipt"].includes(arg)) {
      index += 1;
      continue;
    }
    if (["--platform=", "--out=", "--observation-receipt="].some((prefix) => arg.startsWith(prefix))) continue;
    throw new Error(`unknown argument ${JSON.stringify(arg)}`);
  }

  const inventory = JSON.parse(readFileSync(inventoryPath, "utf8"));
  const featureMap = JSON.parse(readFileSync(featureMapPath, "utf8"));
  const selected = valueArg(args, "--platform") || "all";
  const platforms = selected === "all" ? ["windows", "ubuntu"] : [selected];
  if (platforms.some((platform) => !["windows", "ubuntu"].includes(platform))) {
    throw new Error("--platform must be all, windows, or ubuntu");
  }
  const outputArg = valueArg(args, "--out");
  const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
  const outputDir = outputArg
    ? path.resolve(repoDir, outputArg)
    : path.join(repoDir, "target", "utm-real-use", `${stamp}-${process.pid}`);
  try {
    accessSync(outputDir);
    throw new Error(`refusing to overwrite UTM receipt directory ${outputDir}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);

  const observationPath = valueArg(args, "--observation-receipt");
  let observations;
  let observationMode;
  if (observationPath) {
    observations = JSON.parse(
      readFileSync(path.resolve(repoDir, observationPath), "utf8"),
    ).guests || {};
    observationMode = "supplied_receipt";
  } else {
    observations = {};
    observationMode = "autonomous_macos_window_capture";
    const observationDir = path.join(outputDir, "observations");
    for (const platform of platforms) {
      const environment = inventory.real_use_environments[platform];
      try {
        observations[platform] = await captureUtmWindowObservation({
          platform,
          vmName: environment.vm_name,
          qmpPort: environment.autonomy?.qmp_port,
          outputDir: observationDir,
        });
      } catch (error) {
        observations[platform] = {
          gui_visible: false,
          finding: `autonomous UTM observation failed: ${error.message}`,
        };
      }
    }
  }
  const list = result("utmctl", ["list"]);
  const registrations = list.ok ? parseUtmList(list.stdout) : [];
  const guests = platforms.map((platform) => {
    const environment = inventory.real_use_environments[platform];
    const registration = registrations.find((guest) => guest.name === environment.vm_name);
    const configuration = inspectConfiguration(environment.vm_name);
    const probe = probeGuest(platform, environment.vm_name, registration?.status);
    const evaluated = evaluateGuest({
      platform,
      environment,
      registration,
      configuration,
      probe,
      observation: observations[platform],
    });
    return {
      ...evaluated,
      required_evidence: environment.required_evidence,
      scenarios: [
        ...featureMap.real_use_scenarios[platform],
        ...inventory.real_use_scenarios[platform],
      ].map(({ id, covers, expected }) => ({ id, covers, expected, status: "not_run" })),
    };
  });
  const ready = list.ok && guests.every((guest) => guest.status === "ready");
  const receipt = {
    schema_version: 1,
    benchmark: "clark_code_utm_environment_preflight",
    phase: "environment_preflight",
    status: ready ? "ready" : "blocked",
    generated_at: new Date().toISOString(),
    virtualization: "utm",
    host_platform: process.platform,
    utmctl: { status: list.ok ? "passed" : "failed", error: list.ok ? null : list.stderr },
    credential_recorded: false,
    required_user_vm_actions: 0,
    observation_mode: observationMode,
    guests,
    real_scenarios_executed: 0,
  };
  const receiptPath = path.join(outputDir, "receipt.json");
  const reportPath = path.join(outputDir, "report.md");
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
  writeFileSync(
    reportPath,
    `# Clark Code UTM environment preflight

**Result:** ${receipt.status}
**Virtualization:** UTM
**Real scenarios executed:** 0

| Guest | Environment | Product installed |
| --- | --- | --- |
${guests.map((guest) => `| ${guest.platform} | ${guest.status} | ${guest.product_installed} |`).join("\n")}

This is a readiness receipt, not a feature-test pass. Every real scenario stays
\`not_run\` until its guest is installed, observable, and reachable through the
UTM guest-agent command channel.
`,
    { mode: 0o600 },
  );
  console.log(JSON.stringify({
    status: receipt.status,
    guests: Object.fromEntries(guests.map((guest) => [guest.platform, guest.status])),
  }));
  console.log(`RECEIPT=${receiptPath}`);
  if (!ready && !args.includes("--allow-blocked")) process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await runCli();
}
