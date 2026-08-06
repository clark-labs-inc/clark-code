import { randomBytes } from "node:crypto";

function sleep(milliseconds) {
  if (milliseconds > 0) {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
  }
}

function safeToken(value, label) {
  if (!/^[A-Za-z0-9_-]+$/.test(value)) {
    throw new Error(`${label} contains unsafe characters`);
  }
  return value;
}

function powershellLiteral(value) {
  return `'${String(value).replaceAll("'", "''")}'`;
}

function redactDiagnostic(value) {
  return String(value)
    .replace(/\bck_(?:live|test)_[A-Za-z0-9._-]+\b/g, "ck_[REDACTED]")
    .replace(/\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/g, "[JWT_REDACTED]")
    .replace(/(authorization\s*[:=]\s*bearer\s+)\S+/gi, "$1[REDACTED]")
    .slice(-4_000);
}

export function parseGuestJson(source, marker) {
  const lines = String(source).replace(/^\uFEFF/, "").trim().split(/\r?\n/).reverse();
  for (const line of lines) {
    try {
      const value = JSON.parse(line);
      if (
        value
        && typeof value === "object"
        && !Array.isArray(value)
        && value.probe_marker === marker
      ) {
        return value;
      }
    } catch {
      // A missing file or UTM diagnostic can precede or replace the payload.
    }
  }
  return null;
}

export function buildGuestProbe({
  platform,
  probeSource,
  marker,
  basename,
}) {
  safeToken(marker, "probe marker");
  safeToken(basename, "probe basename");
  if (platform === "ubuntu" || platform === "macos") {
    const outputPath = `/var/tmp/${basename}.json`;
    const scriptPath = `/var/tmp/${basename}.py`;
    const logPath = `/var/tmp/${basename}.log`;
    const guardedSource = String(probeSource)
      .split("\n")
      .map((line) => `    ${line}`)
      .join("\n");
    return {
      outputPath,
      scriptPath,
      scriptContent: `import json, pathlib, sys
try:
${guardedSource}
except Exception as error:
    payload = {
        "guest_probe_failed": True,
        "guest_probe_error_type": type(error).__name__,
        "guest_probe_error": str(error)[:512],
    }
payload["probe_marker"] = sys.argv[2]
pathlib.Path(sys.argv[1]).write_text(json.dumps(payload, separators=(",", ":")), encoding="utf-8")
`,
      command: [
        "/usr/bin/python3",
        scriptPath,
        outputPath,
        marker,
      ],
      detachedCommand: [
        "/bin/sh",
        "-c",
        `nohup /usr/bin/python3 ${scriptPath} ${outputPath} ${marker} >${logPath} 2>&1 </dev/null & printf '%s' $!`,
      ],
      cleanupCommand: ["/usr/bin/rm", "-f", outputPath, scriptPath, logPath],
    };
  }
  if (platform === "windows") {
    const outputPath = `C:\\Users\\Public\\${basename}.json`;
    const scriptPath = `C:\\Users\\Public\\${basename}.ps1`;
    const script = [
      "$ErrorActionPreference = \"Stop\"",
      "$outputPath = " + powershellLiteral(outputPath),
      "try {",
      probeSource,
      "} catch {",
      "  $payload = [ordered]@{",
      "    guest_probe_failed = $true",
      "    guest_probe_error_type = $_.Exception.GetType().FullName",
      "    guest_probe_error = [string]$_.Exception.Message",
      "  }",
      "}",
      `$payload["probe_marker"] = ${powershellLiteral(marker)}`,
      "$payload | ConvertTo-Json -Compress -Depth 20 | Set-Content -LiteralPath $outputPath -Encoding UTF8",
    ].join("\n");
    return {
      outputPath,
      scriptPath,
      scriptContent: `${script}\n`,
      command: [
        "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        scriptPath,
      ],
      detachedCommand: [
        "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        [
          "$process = Start-Process",
          `-FilePath ${powershellLiteral("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe")}`,
          "-ArgumentList @(",
          [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            scriptPath,
          ].map(powershellLiteral).join(","),
          ")",
          "-WindowStyle Hidden -PassThru",
          "; [Console]::Out.Write([string]$process.Id)",
        ].join(" "),
      ],
      cleanupCommand: [
        "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        `Remove-Item -LiteralPath ${powershellLiteral(outputPath)},${powershellLiteral(scriptPath)} -Force -ErrorAction SilentlyContinue`,
      ],
    };
  }
  throw new Error(`unsupported UTM guest platform ${JSON.stringify(platform)}`);
}

export function executeGuestJson({
  platform,
  vmName,
  state,
  probeSource,
  run,
  timeoutMs = 20_000,
  pollAttempts = 60,
  pollDelayMs = 250,
  marker = randomBytes(16).toString("hex"),
  executionAttempts = 2,
  detached = false,
}) {
  if (state !== "started") {
    return { ok: false, data: null, error: `guest state is ${state || "unknown"}, not started` };
  }
  if (!Number.isInteger(executionAttempts) || executionAttempts < 1 || executionAttempts > 3) {
    throw new Error("executionAttempts must be an integer from 1 through 3");
  }
  let lastDiagnostic = "";
  let cleanupSucceeded = null;
  for (let executionAttempt = 0; executionAttempt < executionAttempts; executionAttempt += 1) {
    if (executionAttempt > 0) sleep(pollDelayMs);
    const basename = `clark-qa-probe-${randomBytes(8).toString("hex")}`;
    const probe = buildGuestProbe({ platform, probeSource, marker, basename });
    if (detached && !probe.detachedCommand) {
      throw new Error(`detached guest execution is unsupported for ${platform}`);
    }
    const pushed = run(
      "utmctl",
      ["file", "push", vmName, probe.scriptPath],
      { timeout_ms: timeoutMs, input: probe.scriptContent },
    );
    if (!pushed.ok) {
      lastDiagnostic = pushed.stderr || pushed.stdout || "UTM guest probe push failed";
      continue;
    }
    let scriptReady = false;
    for (let readyAttempt = 0; readyAttempt < 3 && !scriptReady; readyAttempt += 1) {
      if (readyAttempt > 0) sleep(pollDelayMs);
      const pulledScript = run(
        "utmctl",
        ["file", "pull", vmName, probe.scriptPath],
        { timeout_ms: timeoutMs },
      );
      scriptReady = (
        pulledScript.ok
        && String(pulledScript.stdout) === probe.scriptContent
      );
      if (!scriptReady) {
        lastDiagnostic = pulledScript.stderr
          || "UTM guest probe script did not match its pushed bytes";
      }
    }
    if (!scriptReady) {
      const cleanup = run(
        "utmctl",
        ["exec", vmName, "--cmd", ...probe.cleanupCommand],
        { timeout_ms: timeoutMs },
      );
      cleanupSucceeded = cleanup.ok;
      continue;
    }
    const executed = run(
      "utmctl",
      ["exec", vmName, "--cmd", ...(detached ? probe.detachedCommand : probe.command)],
      { timeout_ms: timeoutMs },
    );
    lastDiagnostic = executed.stderr || executed.stdout || "";
    let data = null;
    if (executed.ok || detached) {
      for (let attempt = 0; attempt < pollAttempts && !data; attempt += 1) {
        if (attempt > 0) sleep(pollDelayMs);
        const pulled = run(
          "utmctl",
          ["file", "pull", vmName, probe.outputPath],
          { timeout_ms: timeoutMs },
        );
        data = parseGuestJson(pulled.stdout, marker);
        if (!data) lastDiagnostic = pulled.stderr || pulled.stdout || lastDiagnostic;
      }
    }
    if (data || !detached) {
      const cleanup = run(
        "utmctl",
        ["exec", vmName, "--cmd", ...probe.cleanupCommand],
        { timeout_ms: timeoutMs },
      );
      cleanupSucceeded = cleanup.ok;
    }
    if (data?.guest_probe_failed) {
      return {
        ok: false,
        data: null,
        error: redactDiagnostic(
          `${data.guest_probe_error_type || "GuestProbeError"}: `
          + `${data.guest_probe_error || "guest probe failed"}`,
        ),
        attempts: executionAttempt + 1,
        cleanup_succeeded: cleanupSucceeded,
      };
    }
    if (data) {
      return {
        ok: true,
        data,
        error: null,
        attempts: executionAttempt + 1,
        cleanup_succeeded: cleanupSucceeded,
      };
    }
  }
  return {
    ok: false,
    data: null,
    error: redactDiagnostic(
      lastDiagnostic || (
        detached
          ? "Detached UTM guest job did not produce an authenticated JSON file"
          : "UTM guest probe did not produce an authenticated JSON file"
      ),
    ),
    attempts: executionAttempts,
    cleanup_succeeded: cleanupSucceeded,
  };
}
