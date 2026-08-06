import { spawnSync } from "node:child_process";
import { homedir } from "node:os";
import path from "node:path";

export const UTM_DOCUMENTS_ROOT = path.join(
  homedir(),
  "Library",
  "Containers",
  "com.utmapp.UTM",
  "Data",
  "Documents",
);

function appleScriptString(value) {
  return `"${String(value).replaceAll("\\", "\\\\").replaceAll('"', '\\"')}"`;
}

function safeVmName(name) {
  if (
    typeof name !== "string"
    || !name
    || name.includes("/")
    || name.includes("\\")
    || name === "."
    || name === ".."
  ) {
    throw new Error(`unsafe UTM VM name ${JSON.stringify(name)}`);
  }
  return name;
}

export function readUtmConfig(vmName, root = UTM_DOCUMENTS_ROOT) {
  safeVmName(vmName);
  const configPath = path.join(root, `${vmName}.utm`, "config.plist");
  const converted = spawnSync("plutil", ["-convert", "json", "-o", "-", configPath], {
    encoding: "utf8",
    timeout: 10_000,
  });
  if (converted.status !== 0) {
    throw new Error(converted.stderr || `cannot read UTM configuration for ${vmName}`);
  }
  return JSON.parse(converted.stdout);
}

export function qemuArgumentStrings(config) {
  return (config.QEMU?.AdditionalArguments || []).map(
    (argument) => argument.ArgumentString ?? argument,
  );
}

export function setQemuAdditionalArguments(vmName, entries) {
  safeVmName(vmName);
  if (!Array.isArray(entries) || entries.some((entry) => typeof entry?.value !== "string")) {
    throw new Error("QEMU additional arguments must be { value, filePath? } records");
  }
  const records = entries.map((entry) => {
    const argument = `argument string:${appleScriptString(entry.value)}`;
    if (!entry.filePath) return `{${argument}}`;
    const filePath = path.resolve(entry.filePath);
    return `{${argument}, file urls:{POSIX file ${appleScriptString(filePath)}}}`;
  });
  const script = `tell application "UTM"
  set updatedArguments to {${records.join(", ")}}
  set theVM to first virtual machine whose name is ${appleScriptString(vmName)}
  set theConfiguration to configuration of theVM
  set qemu additional arguments of theConfiguration to updatedArguments
  update configuration theVM with theConfiguration
end tell
`;
  const completed = spawnSync("osascript", ["-"], {
    encoding: "utf8",
    input: script,
    timeout: 20_000,
  });
  const observed = qemuArgumentStrings(readUtmConfig(vmName));
  const expected = entries.map((entry) => entry.value);
  if (
    observed.length !== expected.length
    || observed.some((value, index) => value !== expected[index])
  ) {
    throw new Error(
      completed.stderr
      || `UTM rejected QEMU arguments for ${vmName}; observed ${JSON.stringify(observed)}`,
    );
  }
  return { updated: true, arguments: observed };
}

export function bundleDataPath(vmName, ...parts) {
  safeVmName(vmName);
  return path.join(UTM_DOCUMENTS_ROOT, `${vmName}.utm`, "Data", ...parts);
}

function replaceRemovableDrive(vmName, driveIndex, sourcePath) {
  const configDrive = (readUtmConfig(vmName).Drive || [])[driveIndex - 1];
  if (!configDrive?.ReadOnly || configDrive.ImageType !== "CD") {
    throw new Error(`target drive ${driveIndex} is not removable`);
  }
  const sourceLine = sourcePath
    ? `set replacementDrive to {id:driveId, source:POSIX file ${appleScriptString(path.resolve(sourcePath))}}`
    : "set replacementDrive to {removable:true}";
  const script = `tell application "UTM"
  set theVM to first virtual machine whose name is ${appleScriptString(vmName)}
  set theConfiguration to configuration of theVM
  set configuredDrives to drives of theConfiguration
  set driveId to id of item ${driveIndex} of configuredDrives
  if removable of item ${driveIndex} of configuredDrives is not true then error "target drive is not removable"
  ${sourceLine}
  set item ${driveIndex} of configuredDrives to replacementDrive
  set drives of theConfiguration to configuredDrives
  update configuration theVM with theConfiguration
end tell
`;
  const completed = spawnSync("osascript", ["-"], {
    encoding: "utf8",
    input: script,
    timeout: 20_000,
  });
  if (completed.status !== 0) {
    throw new Error(completed.stderr || `cannot update removable media for ${vmName}`);
  }
  const updatedDrives = readUtmConfig(vmName).Drive || [];
  const retained = updatedDrives.find((drive) => drive.Identifier === configDrive.Identifier);
  if (sourcePath === null && retained) {
    throw new Error(`UTM retained stale removable media identity ${configDrive.Identifier}`);
  }
  if (
    sourcePath !== null
    && (!retained?.ReadOnly || retained.ImageType !== "CD")
  ) {
    throw new Error(`UTM did not retain mounted removable drive ${driveIndex}`);
  }
  return { updated: true, mounted: sourcePath !== null };
}

function removableDriveIndexes(config) {
  return (config.Drive || [])
    .map((drive, index) => ({ drive, index: index + 1 }))
    .filter(({ drive }) => drive.ReadOnly && drive.ImageType === "CD")
    .map(({ index }) => index);
}

export function setRemovableMediaSource(vmName, mediaSlot, sourcePath = null) {
  safeVmName(vmName);
  if (!Number.isInteger(mediaSlot) || mediaSlot < 1) {
    throw new Error("UTM removable-media slot must be a positive integer");
  }
  const config = readUtmConfig(vmName);
  const driveIndex = removableDriveIndexes(config)[mediaSlot - 1];
  if (!driveIndex) {
    throw new Error(`UTM removable-media slot ${mediaSlot} is absent`);
  }
  return replaceRemovableDrive(vmName, driveIndex, sourcePath);
}

export function ensureRemovableMediaSlot(vmName, mediaSlot, sourcePath) {
  safeVmName(vmName);
  if (!Number.isInteger(mediaSlot) || mediaSlot < 1) {
    throw new Error("UTM removable-media slot must be a positive integer");
  }
  const config = readUtmConfig(vmName);
  const removableCount = removableDriveIndexes(config).length;
  if (removableCount >= mediaSlot) {
    return setRemovableMediaSource(vmName, mediaSlot, sourcePath);
  }
  if (removableCount !== mediaSlot - 1) {
    throw new Error(`cannot create non-contiguous UTM removable-media slot ${mediaSlot}`);
  }
  const script = `tell application "UTM"
  set theVM to first virtual machine whose name is ${appleScriptString(vmName)}
  set theConfiguration to configuration of theVM
  set configuredDrives to drives of theConfiguration
  set end of configuredDrives to {removable:true, source:POSIX file ${appleScriptString(path.resolve(sourcePath))}}
  set drives of theConfiguration to configuredDrives
  update configuration theVM with theConfiguration
end tell
`;
  const completed = spawnSync("osascript", ["-"], {
    encoding: "utf8",
    input: script,
    timeout: 20_000,
  });
  if (completed.status !== 0) {
    throw new Error(completed.stderr || `cannot add removable media for ${vmName}`);
  }
  if (removableDriveIndexes(readUtmConfig(vmName)).length !== mediaSlot) {
    throw new Error(`UTM did not create removable-media slot ${mediaSlot}`);
  }
  return { updated: true, mounted: true, created: true };
}

export function resetRemovableMedia(vmName) {
  safeVmName(vmName);
  const before = readUtmConfig(vmName);
  const identities = removableDriveIndexes(before).map(
    (driveIndex) => before.Drive[driveIndex - 1].Identifier,
  );
  for (const identity of identities) {
    const current = readUtmConfig(vmName);
    const driveIndex = (current.Drive || []).findIndex(
      (drive) => drive.Identifier === identity,
    ) + 1;
    if (driveIndex < 1) throw new Error(`UTM lost removable drive ${identity}`);
    replaceRemovableDrive(vmName, driveIndex, null);
  }
  const after = readUtmConfig(vmName);
  const retained = new Set((after.Drive || []).map((drive) => drive.Identifier));
  if (identities.some((identity) => retained.has(identity))) {
    throw new Error("UTM retained a stale removable-media identity");
  }
  if (removableDriveIndexes(after).length !== identities.length) {
    throw new Error("UTM changed the removable-media slot count during reset");
  }
  return { updated: true, cleared: identities.length };
}

export function ejectRemovableMedia(vmName, sourceBasenames) {
  safeVmName(vmName);
  if (
    !Array.isArray(sourceBasenames)
    || sourceBasenames.length === 0
    || sourceBasenames.some(
      (basename) => (
        typeof basename !== "string"
        || !basename
        || basename !== path.basename(basename)
      ),
    )
  ) {
    throw new Error("UTM removable media must be identified by safe basenames");
  }
  const names = sourceBasenames.map(appleScriptString).join(", ");
  const script = `set requestedMedia to {${names}}
tell application "System Events"
  tell process "UTM"
    set frontmost to true
    if not (exists window ${appleScriptString(vmName)}) then error "target VM window is absent"
    perform action "AXRaise" of window ${appleScriptString(vmName)}
    repeat with targetSheet in sheets of window ${appleScriptString(vmName)}
      if exists button "OK" of targetSheet then click button "OK" of targetSheet
    end repeat
    repeat with requestedName in requestedMedia
      set didEject to false
      click menu bar item "Virtual Machine" of menu bar 1
      delay 0.2
      set drivesMenu to menu 1 of menu item "Drives" of menu 1 of menu bar item "Virtual Machine" of menu bar 1
      repeat with driveItem in menu items of drivesMenu
        if (name of driveItem as text) ends with (requestedName as text) then
          if exists menu item "Eject" of menu 1 of driveItem then
            perform action "AXPress" of menu item "Eject" of menu 1 of driveItem
            set didEject to true
            exit repeat
          end if
        end if
      end repeat
      if didEject is false then
        key code 53
        error "requested removable medium is not mounted"
      end if
      delay 0.5
    end repeat
    repeat with requestedName in requestedMedia
      click menu bar item "Virtual Machine" of menu bar 1
      delay 0.2
      set drivesMenu to menu 1 of menu item "Drives" of menu 1 of menu bar item "Virtual Machine" of menu bar 1
      set stillMounted to false
      repeat with driveItem in menu items of drivesMenu
        if (name of driveItem as text) ends with (requestedName as text) then
          set stillMounted to true
          exit repeat
        end if
      end repeat
      key code 53
      if stillMounted then error "removable medium remained mounted after eject"
    end repeat
  end tell
end tell
`;
  const completed = spawnSync("osascript", ["-"], {
    encoding: "utf8",
    input: script,
    timeout: 20_000,
  });
  if (completed.status !== 0) {
    throw new Error(completed.stderr || `cannot eject removable media for ${vmName}`);
  }
  return { updated: true, ejected: sourceBasenames.length };
}
