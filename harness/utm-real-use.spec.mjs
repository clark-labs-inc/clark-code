import assert from "node:assert/strict";
import test from "node:test";

import {
  buildGuestProbe,
  executeGuestJson,
  parseGuestJson,
} from "./utm-guest-channel.mjs";
import { encodeTextToQCodes } from "./utm-qmp.mjs";
import {
  ubuntuOfflineBenchmarkProbe,
  ubuntuPythonParserProbe,
  windowsOfflineBenchmarkProbe,
} from "./utm-guest-benchmark-scripts.mjs";
import {
  ubuntuProvisionProbe,
  windowsPowerShellParserProbe,
  windowsProvisionProbe,
} from "./utm-guest-provision-scripts.mjs";
import {
  ubuntuExtractProbe,
  validateSourcePaths,
  windowsExtractProbe,
} from "./utm-source-stage.mjs";
import {
  buildUbuntuAutoinstall,
  buildWindowsOneShotAutologon,
} from "./utm-unattended-config.mjs";
import { ubuntuBuildInstallLaunchProbe } from "./utm-ubuntu-journey-probe.mjs";
import {
  buildUbuntuAuthenticatedWorkspaceProbe,
} from "./utm-ubuntu-webview.mjs";
import {
  imageLooksGraphical,
  parseWindowList,
} from "./utm-window-observation.mjs";
import {
  mintClarkQaSession,
  parseJwtPayload,
} from "./clark-qa-auth.mjs";
import {
  buildCdpEvaluationProbe,
  buildWebViewPolicyProbe,
  qaAuthenticatedStorageExpression,
  qaStorageExpression,
} from "./utm-windows-webview.mjs";
import {
  evaluateGuest,
  parseJsonTail,
  parseUtmList,
  redact,
  windowsProbe,
} from "./utm-real-use.mjs";

const environments = {
  windows: { vm_name: "Clark QA - Windows 11 ARM" },
  ubuntu: { vm_name: "Clark QA - Ubuntu 24.04 Desktop" },
};

function configuration(platform, allocatedBytes) {
  return {
    ok: true,
    bundle: `${environments[platform].vm_name}.utm`,
    config: {
      Information: { Name: environments[platform].vm_name },
      Display: [{ Hardware: platform === "windows" ? "virtio-ramfb-gl" : "virtio-gpu-pci" }],
      Network: [{ Mode: "Shared" }],
      QEMU: { TPMDevice: platform === "windows" },
    },
    disks: [{ name: "system.qcow2", allocated_bytes: allocatedBytes }],
  };
}

test("UTM list parsing preserves exact UUID, status, and names with spaces", () => {
  const guests = parseUtmList(`UUID                                 Status   Name
95A632BC-CCB1-4EE4-95F0-8AD7609DECF6 started  Clark QA - Windows 11 ARM
F7B555EF-F2BB-463D-9702-9C8BA84C446A stopped  Clark QA - Ubuntu 24.04 Desktop
`);
  assert.deepEqual(guests, [
    {
      uuid: "95A632BC-CCB1-4EE4-95F0-8AD7609DECF6",
      status: "started",
      name: "Clark QA - Windows 11 ARM",
    },
    {
      uuid: "F7B555EF-F2BB-463D-9702-9C8BA84C446A",
      status: "stopped",
      name: "Clark QA - Ubuntu 24.04 Desktop",
    },
  ]);
});

test("guest JSON is recovered from a diagnostic-prefixed UTM response", () => {
  assert.deepEqual(
    parseJsonTail("diagnostic line\n{\"os_id\":\"ubuntu\",\"live_session\":false}\n"),
    { os_id: "ubuntu", live_session: false },
  );
  assert.equal(parseJsonTail("diagnostic only"), null);
});

test("guest file payload must carry the unpredictable probe marker", () => {
  assert.deepEqual(
    parseGuestJson('diagnostic\n{"probe_marker":"expected","ready":true}\n', "expected"),
    { probe_marker: "expected", ready: true },
  );
  assert.equal(
    parseGuestJson('{"probe_marker":"stale","ready":true}\n', "expected"),
    null,
  );
});

test("guest probe paths and commands are platform-scoped and contain no credential", () => {
  const ubuntu = buildGuestProbe({
    platform: "ubuntu",
    probeSource: "payload = {}",
    marker: "marker",
    basename: "probe",
  });
  assert.equal(ubuntu.outputPath, "/var/tmp/probe.json");
  assert.equal(ubuntu.scriptPath, "/var/tmp/probe.py");
  assert.equal(ubuntu.command[0], "/usr/bin/python3");
  assert.equal(ubuntu.detachedCommand[0], "/bin/sh");
  assert.match(ubuntu.detachedCommand.at(-1), /nohup \/usr\/bin\/python3/);

  const windows = buildGuestProbe({
    platform: "windows",
    probeSource: "$payload = [ordered]@{}",
    marker: "marker",
    basename: "probe",
  });
  assert.equal(windows.outputPath, "C:\\Users\\Public\\probe.json");
  assert.equal(windows.scriptPath, "C:\\Users\\Public\\probe.ps1");
  assert.equal(windows.command[0].endsWith("powershell.exe"), true);
  assert.equal(windows.detachedCommand[0].endsWith("powershell.exe"), true);
  assert.match(windows.detachedCommand.at(-1), /Start-Process/);
  assert.match(windows.detachedCommand.at(-1), /-WindowStyle Hidden -PassThru/);
  assert.match(windows.scriptContent, /guest_probe_failed/);
});

test("UTM exec exit zero is not success without an authenticated pulled file", () => {
  const calls = [];
  let scriptPath = "";
  let scriptContent = "";
  const failed = executeGuestJson({
    platform: "ubuntu",
    vmName: environments.ubuntu.vm_name,
    state: "started",
    probeSource: "payload = {}",
    marker: "expected",
    pollAttempts: 1,
    run(command, args, options = {}) {
      calls.push([command, ...args]);
      if (args[0] === "file" && args[1] === "push") {
        scriptPath = args[3];
        scriptContent = options.input;
      }
      return {
        ok: true,
        exit_code: 0,
        stdout: args[0] === "file" && args[1] === "pull" && args[3] === scriptPath
          ? scriptContent
          : args[0] === "file"
            ? "QEMU guest agent is not running"
            : "",
        stderr: "",
      };
    },
  });
  assert.equal(failed.ok, false);
  assert.match(failed.error, /guest agent is not running/);
  assert.equal(calls.some((call) => call[1] === "file" && call[2] === "pull"), true);
});

test("UTM file-channel probe succeeds only with the matching marker", () => {
  let scriptPath = "";
  let scriptContent = "";
  const passed = executeGuestJson({
    platform: "windows",
    vmName: environments.windows.vm_name,
    state: "started",
    probeSource: "$payload = [ordered]@{ os_caption = \"Microsoft Windows 11 Pro\" }",
    marker: "expected",
    pollAttempts: 1,
    run(_command, args, options = {}) {
      if (args[0] === "file" && args[1] === "push") {
        scriptPath = args[3];
        scriptContent = options.input;
      }
      return {
        ok: true,
        exit_code: 0,
        stdout: args[0] === "file" && args[1] === "pull" && args[3] === scriptPath
          ? scriptContent
          : args[0] === "file"
            ? '{"probe_marker":"expected","os_caption":"Microsoft Windows 11 Pro"}'
            : "",
        stderr: "",
      };
    },
  });
  assert.equal(passed.ok, true);
  assert.equal(passed.data.os_caption, "Microsoft Windows 11 Pro");
  assert.equal(passed.cleanup_succeeded, true);
});

test("guest execution waits for an exact pushed-script read-back", () => {
  const calls = [];
  let scriptPath = "";
  let scriptContent = "";
  let scriptPulls = 0;
  const passed = executeGuestJson({
    platform: "windows",
    vmName: environments.windows.vm_name,
    state: "started",
    probeSource: "$payload = [ordered]@{ ready = $true }",
    marker: "expected",
    pollAttempts: 1,
    executionAttempts: 1,
    run(_command, args, options = {}) {
      calls.push(args);
      if (args[0] === "file" && args[1] === "push") {
        scriptPath = args[3];
        scriptContent = options.input;
      }
      if (args[0] === "file" && args[1] === "pull" && args[3] === scriptPath) {
        scriptPulls += 1;
        return {
          ok: true,
          exit_code: 0,
          stdout: scriptPulls === 3 ? scriptContent : "partial script",
          stderr: "",
        };
      }
      return {
        ok: true,
        exit_code: 0,
        stdout: args[0] === "file" && args[1] === "pull"
          ? '{"probe_marker":"expected","ready":true}'
          : "",
        stderr: "",
      };
    },
  });
  assert.equal(passed.ok, true);
  assert.equal(scriptPulls, 3);
  const firstExec = calls.findIndex((args) => args[0] === "exec");
  const thirdScriptPull = calls.reduce(
    (found, args, index) => (
      args[0] === "file" && args[1] === "pull" && args[3] === scriptPath
        ? index
        : found
    ),
    -1,
  );
  assert.equal(firstExec > thirdScriptPull, true);
});

test("missing guest output is retried without human recovery", () => {
  let pushes = 0;
  let scriptPath = "";
  let scriptContent = "";
  const passed = executeGuestJson({
    platform: "windows",
    vmName: environments.windows.vm_name,
    state: "started",
    probeSource: "$payload = [ordered]@{ ready = $true }",
    marker: "expected",
    pollAttempts: 1,
    executionAttempts: 2,
    run(_command, args, options = {}) {
      if (args[0] === "file" && args[1] === "push") {
        pushes += 1;
        scriptPath = args[3];
        scriptContent = options.input;
      }
      return {
        ok: true,
        exit_code: 0,
        stdout: args[0] === "file" && args[1] === "pull" && args[3] === scriptPath
          ? scriptContent
          : args[0] === "file" && args[1] === "pull" && pushes === 2
            ? '{"probe_marker":"expected","ready":true}'
            : "",
        stderr: "",
      };
    },
  });
  assert.equal(passed.ok, true);
  assert.equal(passed.attempts, 2);
  assert.equal(pushes, 2);
});

test("detached Windows execution polls the signed result after a short launcher exits", () => {
  const calls = [];
  let scriptPath = "";
  let scriptContent = "";
  const passed = executeGuestJson({
    platform: "windows",
    vmName: environments.windows.vm_name,
    state: "started",
    probeSource: "$payload = [ordered]@{ ready = $true }",
    marker: "expected",
    pollAttempts: 1,
    executionAttempts: 1,
    detached: true,
    run(_command, args, options = {}) {
      calls.push(args);
      if (args[0] === "file" && args[1] === "push") {
        scriptPath = args[3];
        scriptContent = options.input;
      }
      if (args[0] === "exec" && args.join(" ").includes("Start-Process")) {
        return {
          ok: false,
          exit_code: 1,
          stdout: "",
          stderr: "transient UTM launcher response",
        };
      }
      return {
        ok: true,
        exit_code: 0,
        stdout: args[0] === "file" && args[1] === "pull" && args[3] === scriptPath
          ? scriptContent
          : args[0] === "file" && args[1] === "pull"
            ? '{"probe_marker":"expected","ready":true}'
            : "",
        stderr: "",
      };
    },
  });
  assert.equal(passed.ok, true);
  assert.equal(passed.data.ready, true);
  assert.match(calls.find((args) => args[0] === "exec")[3], /powershell\.exe$/i);
  assert.equal(calls.some((args) => args.join(" ").includes("Start-Process")), true);
});

test("Windows product probe keeps one executable as an array", () => {
  assert.match(
    windowsProbe,
    /\$clarkExecutables = @\(\s+@\([\s\S]*?\)\s+\|\s+Select-Object -ExpandProperty FullName -Unique\s+\)/,
  );
});

test("QMP text encoding handles credentials without retaining or logging them", () => {
  assert.deepEqual(
    encodeTextToQCodes("Aa0! _-/"),
    [
      ["shift", "a"],
      ["a"],
      ["0"],
      ["shift", "1"],
      ["spc"],
      ["shift", "minus"],
      ["minus"],
      ["slash"],
    ],
  );
  assert.throws(() => encodeTextToQCodes("snowman ☃"), /U\+2603/);
});

test("Ubuntu Desktop autoinstall contains integration and only a password hash", () => {
  const config = buildUbuntuAutoinstall({
    username: "home",
    passwordHash: "$6$salt$not-a-real-hash",
  });
  assert.match(config, /id: ubuntu-desktop/);
  assert.match(config, /qemu-guest-agent/);
  assert.match(config, /spice-vdagent/);
  assert.match(config, /AutomaticLogin=home/);
  assert.match(config, /gnome-initial-setup-done/);
  assert.match(config, /shutdown: poweroff/);
  assert.doesNotMatch(config, /test-secret/);
});

test("Windows one-shot login erases the Winlogon secret after GUI startup", () => {
  const script = buildWindowsOneShotAutologon({
    username: "home",
    password: "test-secret",
  });
  assert.match(script, /AutoLogonCount/);
  assert.match(script, /Remove-ItemProperty -Path \$key -Name DefaultPassword/);
  assert.match(script, /ClarkCodeQA-ClearAutologon/);
  assert.match(script, /Unregister-ScheduledTask/);
});

test("guest provisioning pins official toolchains and verifies their provenance", () => {
  const ubuntu = ubuntuProvisionProbe();
  assert.match(ubuntu, /nodejs\.org\/dist/);
  assert.match(ubuntu, /static\.rust-lang\.org\/rustup\/dist/);
  assert.match(ubuntu, /hashlib\.sha256/);
  assert.match(ubuntu, /libwebkit2gtk-4\.1-dev/);
  assert.match(ubuntu, /bubblewrap/);
  assert.match(ubuntu, /apparmor_restrict_unprivileged_userns/);
  assert.match(ubuntu, /\/usr\/bin\/bwrap flags=\(default_allow\)/);
  assert.match(ubuntu, /bubblewrap_sandbox_ready/);
  assert.doesNotMatch(
    ubuntu,
    /apparmor_restrict_unprivileged_userns[^]*write_text\(\s*["']0/s,
  );
  assert.doesNotMatch(ubuntu, /CLARK_QA_VM_PASSWORD|CLARK_QA_AUTH_PASSWORD/);

  const windows = windowsProvisionProbe();
  assert.match(windows, /nodejs\.org\/dist/);
  assert.match(windows, /git-for-windows\/git\/releases\/latest/);
  assert.match(windows, /static\.rust-lang\.org\/rustup\/dist/);
  assert.match(windows, /Get-AuthenticodeSignature/);
  assert.match(windows, /Microsoft\.VisualStudio\.Workload\.VCTools/);
  assert.match(windows, /Microsoft\.VisualStudio\.Component\.VC\.Tools\.ARM64/);
  assert.match(windows, /Microsoft\.VisualStudio\.ComponentGroup\.NativeDesktop\.Llvm\.Clang/);
  assert.match(windows, /msvc_arm64_tools/);
  assert.doesNotMatch(windows, /CLARK_QA_VM_PASSWORD|CLARK_QA_AUTH_PASSWORD/);
});

test("Ubuntu product journey builds embedded ARM assets and launches without a user", () => {
  const probe = ubuntuBuildInstallLaunchProbe();
  assert.match(probe, /--features", "tauri\/custom-protocol"/);
  assert.match(probe, /source-current\.txt/);
  assert.match(probe, /source_sha256/);
  assert.match(probe, /apt-get", "install", "-y", "-qq", "ripgrep"/);
  assert.match(probe, /loginctl", "unlock-session"/);
  assert.match(probe, /GDK_BACKEND": "x11"/);
  assert.match(probe, /xwininfo", "-root", "-tree"/);
  assert.match(probe, /required_user_vm_actions": 0/);
  assert.doesNotMatch(probe, /CLARK_QA_VM_PASSWORD|customer\.example/i);
});

test("Ubuntu authenticated journey seeds only Clark-owned state and erases its transfer", () => {
  const authSession = {
    user: {
      id: "qa-user",
      name: "Autonomous VM QA",
      email: "clark-code-vm-qa@clarkslabs.com",
      method: "local",
    },
    clark: {
      endpoint: "wss://www.clarkchat.com/ws",
      token: "eyJ0ZXN0Ijp0cnVlfQ.eyJzdWIiOiJxYS11c2VyIn0.signature",
    },
  };
  const probe = buildUbuntuAuthenticatedWorkspaceProbe({ authSession });
  assert.match(probe, /tauri_localhost_0\.localstorage/);
  assert.match(probe, /utf-16-le/);
  assert.match(probe, /provider_key_owner_bound/);
  assert.match(probe, /clark-code:minimax_m3/);
  assert.match(probe, /loginctl", "unlock-session"/);
  assert.match(probe, /required_user_vm_actions": 0/);
  assert.doesNotMatch(probe, /customer\.example|CLARK_QA_AUTH_PASSWORD/i);
  assert.doesNotMatch(probe, new RegExp(authSession.clark.token.replaceAll(".", "\\.")));
  assert.throws(
    () => buildUbuntuAuthenticatedWorkspaceProbe({
      authSession: {
        ...authSession,
        user: {
          ...authSession.user,
          email: "clark-code-vm-qa@customer.example",
        },
      },
    }),
    /Clark-owned clarkslabs\.com domain/,
  );
});

test("Windows provisioning syntax is preflighted without placing raw source on the command line", () => {
  const source = '$payload = [ordered]@{ sentinel = "raw-probe-source" }';
  const parserProbe = windowsPowerShellParserProbe(source);
  assert.match(parserProbe, /Language\.Parser\]::ParseInput/);
  assert.match(parserProbe, /syntax_valid/);
  assert.match(parserProbe, /line = \$_.Extent\.StartLineNumber/);
  assert.match(parserProbe, new RegExp(Buffer.from(source, "utf8").toString("base64")));
  assert.doesNotMatch(parserProbe, /raw-probe-source/);
});

test("UTM guest benchmark scripts pin source, lockfiles, offline mode, and syntax preflights", () => {
  const runId = "offline-0123456789abcdef";
  const windows = windowsOfflineBenchmarkProbe({ runId });
  assert.match(windows, /source-current\.txt/);
  assert.match(windows, /--frozen-lockfile/);
  assert.match(windows, /--offline.*--platform", "windows"/s);
  assert.match(windows, /WaitForExit\(\$TimeoutSeconds \* 1000\)/);
  assert.match(windows, /\$process\.Refresh\(\)/);
  assert.match(windows, /\$exitCode = \[int\]\$process\.ExitCode/);
  assert.match(windows, /-arch=' \+ \$vsArch/);
  assert.match(windows, /PLAYWRIGHT_BROWSERS_PATH/);
  assert.match(windows, /playwright", "install", "--only-shell", "chromium"/);
  assert.doesNotMatch(windows, /CLARK_QA_VM_PASSWORD|CLARK_CODE_API_KEY/);

  const ubuntu = ubuntuOfflineBenchmarkProbe({ runId });
  assert.match(ubuntu, /source-current\.txt/);
  assert.match(ubuntu, /--frozen-lockfile/);
  assert.match(ubuntu, /"--offline", "--platform", "ubuntu"/);
  assert.match(ubuntu, /start_new_session=True/);
  assert.match(ubuntu, /PLAYWRIGHT_BROWSERS_PATH/);
  assert.match(ubuntu, /"playwright", "install", "--only-shell", "chromium"/);
  assert.doesNotMatch(ubuntu, /CLARK_QA_VM_PASSWORD|CLARK_CODE_API_KEY/);

  const parser = ubuntuPythonParserProbe('payload = {"sentinel": "raw-python-source"}');
  assert.match(parser, /compile\(source/);
  assert.doesNotMatch(parser, /raw-python-source/);
  assert.throws(
    () => windowsOfflineBenchmarkProbe({ runId: "../unsafe" }),
    /run id/,
  );
});

test("source staging excludes the Windows-reserved artifact and any real .env", () => {
  const sourceSet = validateSourcePaths([
    "Cargo.toml",
    "NUL",
    "app/package.json",
    "harness/clark-code-feature-map.json",
    "harness/utm-source-stage.mjs",
  ]);
  assert.equal(sourceSet.accepted.includes("NUL"), false);
  assert.deepEqual(sourceSet.excluded, [{
    path: "NUL",
    reason: "Windows-reserved untracked artifact",
  }]);
  assert.throws(
    () => validateSourcePaths([
      ".env",
      "Cargo.toml",
      "app/package.json",
      "harness/clark-code-feature-map.json",
      "harness/utm-source-stage.mjs",
    ]),
    /credential-bearing environment file/,
  );
});

test("source extraction verifies SHA-256 and writes a guest current pointer", () => {
  const hash = "a".repeat(64);
  const windows = windowsExtractProbe({
    archivePath: String.raw`C:\Users\Public\source.tgz`,
    sourceSha256: hash,
  });
  assert.match(windows, /Get-FileHash -Algorithm SHA256/);
  assert.match(windows, /source-current\.txt/);
  assert.match(windows, /env_present/);
  assert.match(windows, /appledouble_count/);

  const ubuntu = ubuntuExtractProbe({
    archivePath: "/var/tmp/source.tgz",
    sourceSha256: hash,
  });
  assert.match(ubuntu, /hashlib\.sha256/);
  assert.match(ubuntu, /source-current\.txt/);
  assert.match(ubuntu, /archive path escape/);
  assert.match(ubuntu, /appledouble_count/);
});

test("Windows WebView control is loopback-only, temporary, and CDP-encoded", () => {
  const enabled = buildWebViewPolicyProbe({ enabled: true, cdpPort: 9222 });
  const disabled = buildWebViewPolicyProbe({ enabled: false, cdpPort: 9222 });
  assert.match(enabled, /--remote-debugging-port=9222/);
  assert.match(enabled, /--remote-allow-origins=http:\/\/127\.0\.0\.1:9222/);
  assert.match(enabled, /RegistryValueKind/);
  assert.match(disabled, /DeleteValue/);

  const expression = `document.body.dataset.secret = "not-on-the-wire"`;
  const cdp = buildCdpEvaluationProbe({ expression, cdpPort: 9222 });
  assert.doesNotMatch(cdp, /not-on-the-wire/);
  assert.match(cdp, /FromBase64String/);
});

test("Windows QA storage fixture has local auth but no Clark cloud token", () => {
  const expression = qaStorageExpression({
    cwd: String.raw`C:\Users\home\ClarkCodeQA`,
    model: "clark-code:minimax_m3",
  });
  assert.match(expression, /local_only_no_cloud_token/);
  assert.match(expression, /clark-code:minimax_m3/);
  assert.doesNotMatch(expression, /ck_live_/);
  assert.doesNotMatch(expression, /"token"/);
});

test("Clark QA login mints a short-lived account-bound session without VM input", async () => {
  const now = Date.parse("2026-07-24T08:00:00Z");
  const encode = (value) => Buffer.from(JSON.stringify(value)).toString("base64url");
  const jwt = [
    encode({ alg: "RS256", typ: "JWT" }),
    encode({
      iss: "https://www.clarkchat.com",
      sub: "qa-user-id",
      exp: Math.floor(now / 1_000) + 900,
    }),
    "test-signature",
  ].join(".");
  const calls = [];
  const fetchImpl = async (url, options = {}) => {
    calls.push({ url, options });
    if (url.endsWith("/api/auth/sign-in/email")) {
      return {
        ok: true,
        status: 200,
        headers: {
          getSetCookie: () => ["better-auth.session_token=opaque; HttpOnly; Secure"],
        },
        json: async () => ({
          user: {
            id: "qa-user-id",
            name: "Autonomous VM QA",
            email: "clark-code-vm-qa@clarkslabs.com",
          },
        }),
      };
    }
    return {
      ok: true,
      status: 200,
      headers: { getSetCookie: () => [] },
      json: async () => ({ token: jwt }),
    };
  };
  const result = await mintClarkQaSession({
    credentials: {
      name: "Autonomous VM QA",
      email: "clark-code-vm-qa@clarkslabs.com",
      password: "not-a-real-password",
    },
    fetchImpl,
    now: () => now,
  });
  assert.equal(calls.length, 2);
  assert.equal(calls[1].options.headers.cookie, "better-auth.session_token=opaque");
  assert.equal(result.session.user.id, "qa-user-id");
  assert.equal(result.session.user.method, "local");
  assert.equal(result.session.clark.endpoint, "wss://www.clarkchat.com/ws");
  assert.equal(result.expires_in_seconds, 900);
  assert.equal(result.required_user_vm_actions, 0);
  assert.equal(result.credential_recorded, false);
  assert.equal(parseJwtPayload(jwt).sub, "qa-user-id");
});

test("authenticated Windows fixture carries no pasted provider key", () => {
  const encode = (value) => Buffer.from(JSON.stringify(value)).toString("base64url");
  const jwt = [
    encode({ alg: "RS256" }),
    encode({ sub: "qa-user-id", exp: 4_102_444_800 }),
    "test-signature",
  ].join(".");
  const expression = qaAuthenticatedStorageExpression({
    authSession: {
      user: {
        id: "qa-user-id",
        name: "Autonomous VM QA",
        email: "clark-code-vm-qa@clarkslabs.com",
        method: "local",
      },
      clark: {
        endpoint: "wss://www.clarkchat.com/ws",
        token: jwt,
      },
    },
  });
  assert.match(expression, /dedicated_qa_short_lived_jwt/);
  assert.doesNotMatch(expression, /ck_live_/);
  const encodedProbe = buildCdpEvaluationProbe({ expression });
  assert.doesNotMatch(encodedProbe, /clark-code-vm-qa@clarkslabs\.com/);
  assert.doesNotMatch(encodedProbe, /test-signature/);
});

test("UTM GUI observation distinguishes a black framebuffer from a desktop", () => {
  assert.equal(imageLooksGraphical({ mean: 0, standardDeviation: 0 }), false);
  assert.equal(imageLooksGraphical({ mean: 0.54, standardDeviation: 0.22 }), true);
  assert.deepEqual(
    parseWindowList('[{"id":8934,"name":"Clark QA - Windows 11 ARM","bounds":{}}]'),
    [{ id: 8934, name: "Clark QA - Windows 11 ARM", bounds: {} }],
  );
});

test("Ubuntu live media stays blocked even when its GUI is visible", () => {
  const evaluated = evaluateGuest({
    platform: "ubuntu",
    environment: environments.ubuntu,
    registration: { uuid: "ubuntu-id", status: "started" },
    configuration: configuration("ubuntu", 3 * 1024 * 1024 * 1024),
    observation: { gui_visible: true, finding: "Ubuntu live desktop visible" },
    probe: {
      ok: true,
      data: {
        os_id: "ubuntu",
        live_session: true,
        root_source: "overlay",
        desktop_target: "graphical.target",
        display_manager_active: true,
        ubuntu_desktop_installed: false,
        spice_agent_installed: true,
        bubblewrap_installed: true,
        qemu_ga_service: "active",
        clark_code_installed: false,
      },
    },
  });
  assert.equal(evaluated.status, "blocked");
  assert.equal(
    evaluated.checks.find((item) => item.id === "installed_not_live_media").status,
    "failed",
  );
});

test("an installed Ubuntu Desktop guest with integration and bubblewrap is ready", () => {
  const evaluated = evaluateGuest({
    platform: "ubuntu",
    environment: environments.ubuntu,
    registration: { uuid: "ubuntu-id", status: "started" },
    configuration: configuration("ubuntu", 8 * 1024 * 1024 * 1024),
    observation: { gui_visible: true, finding: "Ubuntu login desktop verified" },
    probe: {
      ok: true,
      data: {
        os_id: "ubuntu",
        live_session: false,
        root_source: "/dev/vda2",
        desktop_target: "graphical.target",
        display_manager_active: true,
        ubuntu_desktop_installed: true,
        spice_agent_installed: true,
        bubblewrap_installed: true,
        qemu_ga_service: "active",
        initial_setup_done: true,
        initial_setup_running: false,
        clark_code_installed: true,
      },
    },
  });
  assert.equal(evaluated.status, "ready");
  assert.equal(evaluated.product_installed, true);
});

test("Ubuntu post-install welcome blocks readiness without asking the user", () => {
  const evaluated = evaluateGuest({
    platform: "ubuntu",
    environment: environments.ubuntu,
    registration: { uuid: "ubuntu-id", status: "started" },
    configuration: configuration("ubuntu", 8 * 1024 * 1024 * 1024),
    observation: { gui_visible: true, finding: "Ubuntu welcome dialog visible" },
    probe: {
      ok: true,
      data: {
        os_id: "ubuntu",
        live_session: false,
        root_source: "/dev/vda2",
        desktop_target: "graphical.target",
        display_manager_active: true,
        ubuntu_desktop_installed: true,
        spice_agent_installed: true,
        bubblewrap_installed: true,
        qemu_ga_service: "active",
        initial_setup_done: false,
        initial_setup_running: true,
        clark_code_installed: false,
      },
    },
  });
  assert.equal(evaluated.status, "blocked");
  assert.equal(
    evaluated.checks.find((item) => item.id === "initial_setup_complete").status,
    "failed",
  );
});

test("a black Windows console without the guest agent is blocked", () => {
  const evaluated = evaluateGuest({
    platform: "windows",
    environment: environments.windows,
    registration: { uuid: "windows-id", status: "started" },
    configuration: configuration("windows", 35 * 1024 * 1024 * 1024),
    observation: { gui_visible: false, finding: "console is black" },
    probe: { ok: false, data: null, error: "QEMU guest agent is not running" },
  });
  assert.equal(evaluated.status, "blocked");
  assert.equal(
    evaluated.checks.find((item) => item.id === "guest_agent_command_channel").status,
    "failed",
  );
  assert.equal(
    evaluated.checks.find((item) => item.id === "fresh_gui_observation").status,
    "failed",
  );
});

test("a visible Windows 11 desktop with the command channel is ready", () => {
  const evaluated = evaluateGuest({
    platform: "windows",
    environment: environments.windows,
    registration: { uuid: "windows-id", status: "started" },
    configuration: configuration("windows", 35 * 1024 * 1024 * 1024),
    observation: { gui_visible: true, finding: "Windows 11 desktop verified" },
    probe: {
      ok: true,
      data: {
        os_caption: "Microsoft Windows 11 Pro",
        desktop_shell_running: true,
        qemu_ga_service: "Running",
        clark_code_installed: false,
      },
    },
  });
  assert.equal(evaluated.status, "ready");
  assert.equal(evaluated.product_installed, false);
});

test("receipt diagnostics redact provider keys", () => {
  assert.equal(redact("token=ck_live_not-a-real-key"), "token=ck_[REDACTED]");
  assert.equal(redact("Authorization: Bearer secret-value"), "Authorization: Bearer [REDACTED]");
});
