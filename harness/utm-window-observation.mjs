#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  chmodSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { QmpClient } from "./utm-qmp.mjs";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const inventoryPath = path.join(harnessDir, "clark-code-capability-inventory.json");

const windowListSource = String.raw`
import CoreGraphics
import Foundation

let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
let info = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] ?? []
let windows = info.compactMap { item -> [String: Any]? in
    guard (item[kCGWindowOwnerName as String] as? String) == "UTM" else { return nil }
    return [
        "id": item[kCGWindowNumber as String] as? Int ?? 0,
        "name": item[kCGWindowName as String] as? String ?? "",
        "bounds": item[kCGWindowBounds as String] as? [String: Any] ?? [:],
    ]
}
let data = try! JSONSerialization.data(withJSONObject: windows)
print(String(data: data, encoding: .utf8)!)
`;

function run(command, args, options = {}) {
  const completed = spawnSync(command, args, {
    encoding: "utf8",
    timeout: options.timeoutMs ?? 30_000,
    input: options.input,
    maxBuffer: 4 * 1024 * 1024,
  });
  return {
    ok: completed.status === 0,
    status: completed.status,
    stdout: completed.stdout || "",
    stderr: completed.stderr || completed.error?.message || "",
  };
}

function sleep(milliseconds) {
  if (milliseconds > 0) {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
  }
}

export function parseWindowList(source) {
  const value = JSON.parse(String(source).trim());
  if (!Array.isArray(value)) throw new Error("CoreGraphics window inventory is not an array");
  return value.filter(
    (window) => Number.isInteger(window.id) && window.id > 0 && typeof window.name === "string",
  );
}

export function imageLooksGraphical({ mean, standardDeviation }) {
  return (
    Number.isFinite(mean)
    && Number.isFinite(standardDeviation)
    && mean >= 0.03
    && standardDeviation >= 0.015
  );
}

function listUtmWindows() {
  const completed = run("swift", ["-"], { input: windowListSource });
  if (!completed.ok) throw new Error(completed.stderr || "cannot inventory UTM windows");
  return parseWindowList(completed.stdout);
}

function imageStatistics(imagePath) {
  const identified = run("magick", ["identify", "-format", "%w %h", imagePath]);
  if (!identified.ok) throw new Error(identified.stderr || "cannot inspect VM screenshot dimensions");
  const [width, height] = identified.stdout.trim().split(/\s+/).map(Number);
  if (!Number.isInteger(width) || !Number.isInteger(height) || width < 200 || height < 200) {
    throw new Error(`invalid VM screenshot dimensions ${identified.stdout.trim()}`);
  }
  const top = Math.min(110, Math.floor(height / 8));
  const contentHeight = height - top;
  const measured = run("magick", [
    imagePath,
    "-crop",
    `${width}x${contentHeight}+0+${top}`,
    "+repage",
    "-alpha",
    "off",
    "-colorspace",
    "Gray",
    "-format",
    "%[fx:mean] %[fx:standard_deviation]",
    "info:",
  ]);
  if (!measured.ok) throw new Error(measured.stderr || "cannot analyze VM screenshot pixels");
  const [mean, standardDeviation] = measured.stdout.trim().split(/\s+/).map(Number);
  return { width, height, mean, standard_deviation: standardDeviation };
}

async function wakeGuest(qmpPort) {
  if (!qmpPort) return { status: "not_configured" };
  const client = new QmpClient({ port: qmpPort });
  try {
    await client.connect();
    await client.sendChord(["shift"]);
    return { status: "passed", transport: "localhost_qmp" };
  } catch (error) {
    return { status: "failed", transport: "localhost_qmp", error: error.message };
  } finally {
    client.close();
  }
}

function prepareUtmWindow(vmName) {
  const escaped = vmName.replaceAll("\\", "\\\\").replaceAll('"', '\\"');
  const script = `set raised to 0
set dismissed to 0
set wasMinimized to false
set stillMinimized to false
set windowMenuActivated to 0
tell application "UTM" to activate
tell application "System Events"
  tell process "UTM"
    set frontmost to true
    repeat with targetWindow in windows
      if name of targetWindow is "${escaped}" then
        try
          set wasMinimized to value of attribute "AXMinimized" of targetWindow
          set value of attribute "AXMinimized" of targetWindow to false
        end try
        delay 0.2
        try
          set stillMinimized to value of attribute "AXMinimized" of targetWindow
        end try
        if stillMinimized then
          try
            click menu item "${escaped}" of menu 1 of menu bar item "Window" of menu bar 1
            set windowMenuActivated to 1
            delay 0.2
            set stillMinimized to value of attribute "AXMinimized" of targetWindow
          end try
        end if
        try
          perform action "AXRaise" of targetWindow
          set raised to raised + 1
        end try
        repeat with targetSheet in sheets of targetWindow
          if exists button "OK" of targetSheet then
            click button "OK" of targetSheet
            set dismissed to dismissed + 1
          end if
        end repeat
      end if
    end repeat
  end tell
end tell
return (raised as text) & "," & (dismissed as text) & "," & (wasMinimized as text) & "," & (stillMinimized as text) & "," & (windowMenuActivated as text)
`;
  const completed = run("osascript", ["-"], { input: script });
  if (!completed.ok) {
    return {
      status: "failed",
      exact_windows_raised: 0,
      alerts_dismissed: 0,
      error: completed.stderr || "UTM window preparation failed",
    };
  }
  const [raisedText, dismissedText, beforeText, afterText, menuText] =
    completed.stdout.trim().split(",");
  const raised = Number(raisedText);
  const dismissed = Number(dismissedText);
  const stillMinimized = afterText === "true";
  return {
    status: raised === 1 && !stillMinimized ? "passed" : "failed",
    exact_windows_raised: raised || 0,
    alerts_dismissed: dismissed || 0,
    was_minimized: beforeText === "true",
    still_minimized: stillMinimized,
    window_menu_activated: Number(menuText) === 1,
    error: raised === 1 && !stillMinimized
      ? null
      : `could not unminimize and raise the exact UTM window named ${vmName}`,
  };
}

export async function captureUtmWindowObservation({
  platform,
  vmName,
  qmpPort,
  outputDir,
}) {
  if (process.platform !== "darwin") {
    throw new Error("UTM window observation requires the macOS host");
  }
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  chmodSync(outputDir, 0o700);
  const wake = await wakeGuest(qmpPort);
  const preparations = [];
  const screenshotPath = path.join(outputDir, `${platform}.png`);
  let matches = [];
  let captured = null;
  for (let attempt = 0; attempt < 3; attempt += 1) {
    preparations.push(prepareUtmWindow(vmName));
    sleep(attempt === 0 ? 1_000 : 1_500);
    matches = listUtmWindows().filter((window) => window.name === vmName);
    if (matches.length !== 1) continue;
    captured = run("/usr/sbin/screencapture", [
      "-x",
      "-o",
      "-l",
      String(matches[0].id),
      screenshotPath,
    ]);
    if (captured.ok) break;
  }
  const windowPreparation = {
    ...preparations.at(-1),
    attempts: preparations,
  };
  if (matches.length !== 1 || !captured) {
    return {
      gui_visible: false,
      finding: `expected one on-screen UTM window named ${vmName}; found ${matches.length}`,
      wake,
      window_preparation: windowPreparation,
      host_alerts_dismissed: windowPreparation.alerts_dismissed,
    };
  }
  if (!captured.ok) {
    return {
      gui_visible: false,
      finding: captured.stderr || "macOS window capture failed",
      wake,
      window_preparation: windowPreparation,
      host_alerts_dismissed: windowPreparation.alerts_dismissed,
    };
  }
  chmodSync(screenshotPath, 0o600);
  const statistics = imageStatistics(screenshotPath);
  const guiVisible = imageLooksGraphical({
    mean: statistics.mean,
    standardDeviation: statistics.standard_deviation,
  });
  const digest = createHash("sha256").update(readFileSync(screenshotPath)).digest("hex");
  return {
    gui_visible: guiVisible,
    finding: guiVisible
      ? `fresh ${platform} graphical framebuffer captured from the exact UTM window`
      : `fresh ${platform} UTM framebuffer is blank or visually degenerate`,
    observed_at: new Date().toISOString(),
    capture_transport: "macos_window_id",
    wake,
    window_preparation: windowPreparation,
    host_alerts_dismissed: windowPreparation.alerts_dismissed,
    screenshot: path.relative(repoDir, screenshotPath),
    screenshot_sha256: digest,
    image: statistics,
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
    console.log(`Autonomous UTM GUI observation

Usage:
  node harness/utm-window-observation.mjs [--out NEW_DIRECTORY]
    [--platform all|windows|ubuntu]

The command wakes each guest through a localhost-only QMP monitor, captures its
exact UTM window through macOS, rejects blank framebuffers, and writes an
owner-only receipt. It never asks for physical input.`);
    return;
  }
  const known = new Set(["--help", "-h"]);
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (known.has(arg)) continue;
    if (["--out", "--platform"].includes(arg)) {
      index += 1;
      continue;
    }
    if (["--out=", "--platform="].some((prefix) => arg.startsWith(prefix))) continue;
    throw new Error(`unknown argument ${JSON.stringify(arg)}`);
  }
  const outputArg = valueArg(args, "--out")
    || path.join(
      "target",
      "utm-observation",
      `${new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z")}-${process.pid}`,
    );
  const outputDir = path.resolve(repoDir, outputArg);
  const selected = valueArg(args, "--platform") || "all";
  const platforms = selected === "all" ? ["windows", "ubuntu"] : [selected];
  if (platforms.some((platform) => !["windows", "ubuntu"].includes(platform))) {
    throw new Error("--platform must be all, windows, or ubuntu");
  }
  const inventory = JSON.parse(readFileSync(inventoryPath, "utf8"));
  const guests = {};
  for (const platform of platforms) {
    const environment = inventory.real_use_environments[platform];
    guests[platform] = await captureUtmWindowObservation({
      platform,
      vmName: environment.vm_name,
      qmpPort: environment.autonomy?.qmp_port,
      outputDir,
    });
  }
  const receipt = {
    schema_version: 1,
    benchmark: "clark_code_utm_autonomous_gui_observation",
    status: Object.values(guests).every((guest) => guest.gui_visible) ? "passed" : "blocked",
    generated_at: new Date().toISOString(),
    required_user_vm_actions: 0,
    virtualization: "utm",
    guests,
  };
  const receiptPath = path.join(outputDir, "receipt.json");
  writeFileSync(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, { mode: 0o600 });
  console.log(JSON.stringify({
    status: receipt.status,
    guests: Object.fromEntries(
      Object.entries(guests).map(([platform, guest]) => [platform, guest.gui_visible]),
    ),
  }));
  console.log(`RECEIPT=${receiptPath}`);
  if (receipt.status !== "passed") process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await runCli();
}
