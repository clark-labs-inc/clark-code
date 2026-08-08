import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

test("the open workspace has no Clark product packages", () => {
  const metadata = JSON.parse(execFileSync(
    "cargo",
    ["metadata", "--no-deps", "--format-version", "1"],
    { cwd: root, encoding: "utf8" },
  ));
  const packages = new Set(metadata.packages.map(({ name }) => name));
  assert.equal(packages.has("provider-clark"), false);
  assert.equal(packages.has("provider-specialist"), false);
  assert.equal(packages.has("clark-cli"), false);
  assert.equal(packages.has("conversation-cloud"), false);
  assert.equal(packages.has("security-cloud-sync"), false);

  for (const relative of [
    "Cargo.toml",
    "src-tauri/Cargo.toml",
    "crates/provider-local/Cargo.toml",
    "crates/devbridge/Cargo.toml",
  ]) {
    assert.doesNotMatch(
      readFileSync(resolve(root, relative), "utf8"),
      /provider-clark|provider-specialist|clark-cli|conversation-cloud|security-cloud-sync/,
    );
  }
});

test("the neutral Tauri config contains no Clark release authority", () => {
  const config = readFileSync(resolve(root, "src-tauri/tauri.conf.json"), "utf8");
  assert.doesNotMatch(config, /clarkchat|com\.clark|downloads\.clark|\"clark\"/i);
  for (const relative of [
    "src-tauri/tauri.release.conf.json",
    "src-tauri/tauri.windows-signing.conf.json",
    "src-tauri/sign-windows-artifact.ps1",
    "src-tauri/windows/preserve-sandbox-state.nsh",
  ]) {
    assert.equal(spawnSync("test", ["-e", relative], { cwd: root }).status, 1);
  }
});

test("the open bundle overlays do not package the private specialist worker", () => {
  for (const relative of [
    "src-tauri/tauri.computer-use.macos.conf.json",
    "src-tauri/tauri.remote-workers.dev.conf.json",
    "src-tauri/tauri.qa.macos.conf.json",
  ]) {
    assert.equal(spawnSync("test", ["-e", relative], { cwd: root }).status, 1);
  }
  for (const relative of [
    "src-tauri/tauri.sandbox.linux.conf.json",
    "src-tauri/tauri.sandbox.macos.conf.json",
    "src-tauri/tauri.sandbox.windows.conf.json",
  ]) {
    assert.doesNotMatch(
      readFileSync(resolve(root, relative), "utf8"),
      /agent-desktop-headless/,
      `${relative} must not package the private Scientist runtime`,
    );
  }
});

test("billing policy is owned by the downstream product", () => {
  for (const relative of [
    "app/src/lib/billing.ts",
    "app/src/surfaces/BillingStateSync.tsx",
    "app/src/surfaces/CreditBanner.tsx",
  ]) {
    assert.equal(spawnSync("test", ["-e", relative], { cwd: root }).status, 1);
  }
  const native = readFileSync(resolve(root, "src-tauri/src/lib.rs"), "utf8");
  assert.doesNotMatch(native, /clark_billing_me|api\/billing\/me/);
});

test("account sign-in policy is owned by the downstream product", () => {
  assert.equal(
    spawnSync("test", ["-e", "src-tauri/src/commands/auth.rs"], { cwd: root }).status,
    1,
  );
  const rendererAuth = readFileSync(resolve(root, "app/src/lib/auth.ts"), "utf8");
  assert.doesNotMatch(
    rendererAuth,
    /clark_account_load|clark_google_sign_in|clark_refresh_cloud_session|clark_sign_out/,
  );
  assert.match(rendererAuth, /productRequest<.*>\("account\.(load|sign_in|refresh)"\)/);
});

test("first-party specialist policy is owned by the downstream product", () => {
  for (const relative of [
    "app/src/lib/first-party-specialists.json",
    "app/src/surfaces/ClarkMark.tsx",
    "src-tauri/src/commands/specialists.rs",
  ]) {
    assert.equal(spawnSync("test", ["-e", relative], { cwd: root }).status, 1);
  }
  const native = readFileSync(resolve(root, "src-tauri/src/lib.rs"), "utf8");
  assert.doesNotMatch(native, /desktop_specialist_|provider-specialist/);

  const specialistUi = [
    "app/src/lib/specialists.ts",
    "app/src/surfaces/specialists/SpecialistAccessGate.tsx",
    "app/src/surfaces/specialists/SpecialistNavigation.tsx",
  ].map((relative) => readFileSync(resolve(root, relative), "utf8")).join("\n");
  assert.doesNotMatch(specialistUi, /FIRST_PARTY_SPECIALIST_CATALOG|\bPro\b/);
  assert.match(specialistUi, /specialistIcons/);
});

test("the design system has one current token vocabulary", () => {
  const css = readFileSync(resolve(root, "app/src/index.css"), "utf8");
  assert.doesNotMatch(
    css,
    /--color-(?:bg-primary|text-primary|text-secondary|text-muted|text-faint|accent-muted|canvas|surface|primary|focus|on-primary)\s*:/,
  );
  assert.doesNotMatch(css, /Compatibility aliases|Transitional bridge/);

  const appSource = spawnSync(
    "rg",
    ["-n", "text-\\[(?:10px|11px|0\\.6875rem)\\]|bg-bg-primary|bg-accent[^\\n]*text-white", "app/src"],
    { cwd: root, encoding: "utf8" },
  );
  assert.equal(appSource.status, 1, appSource.stdout || appSource.stderr);
});

test("Clark cloud security, artifact, and mobile transports are downstream", () => {
  for (const relative of [
    "crates/security-cloud-sync/Cargo.toml",
    "crates/provider-local/src/security_export.rs",
    "src-tauri/src/commands/security_cloud.rs",
    "src-tauri/src/commands/security_cloud/client_tests.rs",
    "src-tauri/src/mobile_remote.rs",
  ]) {
    assert.equal(spawnSync("test", ["-e", relative], { cwd: root }).status, 1);
  }
  for (const relative of [
    "src-tauri/src/commands/desktop_artifacts.rs",
    "src-tauri/src/lib.rs",
    "crates/provider-local/src/lib.rs",
  ]) {
    assert.doesNotMatch(
      readFileSync(resolve(root, relative), "utf8"),
      /api\/desktop\/code|api\/desktop\/conversations\/.+artifacts|api\/orgs\/.+security|ClarkSecurityCloud/,
    );
  }
});

test("local-agent product policy is injected instead of compiled into the foundation", () => {
  for (const relative of [
    "crates/provider-local/src/config.rs",
    "crates/provider-local/src/configuration.rs",
    "crates/provider-local/src/project_settings.rs",
    "app/src/lib/localAgent.ts",
  ]) {
    assert.doesNotMatch(
      readFileSync(resolve(root, relative), "utf8"),
      /api\.clarkslabs\.com|clarkchat\.com|clark-code:(?:free|glm52|kimi_k3|deepseek)/,
    );
  }
});

test("remote-worker launch policy is owned by the downstream product", () => {
  const source = readFileSync(
    resolve(root, "src-tauri/src/commands/remote_worker.rs"),
    "utf8",
  );
  for (const privateDetail of [
    "api.clarkslabs.com",
    "clark-code:",
    "CLARK_CODE_API_KEY",
    "CODE_REMOTE_LINUX_X86_64",
    "agent-desktop-worker",
  ]) {
    assert.equal(source.includes(privateDetail), false, privateDetail);
  }
  assert.match(source, /prepare_remote_worker/);
});

test("product extensions remain dependency-inverted", () => {
  const integration = readFileSync(resolve(root, "src-tauri/src/product.rs"), "utf8");
  assert.match(integration, /trait ProductIntegration/);
  assert.match(integration, /make_provider/);
  assert.match(integration, /prepare_provider_config/);
  assert.match(integration, /async fn request/);

  const tools = readFileSync(resolve(root, "crates/provider-local/src/tools/mod.rs"), "utf8");
  assert.match(tools, /trait ToolPack/);
  assert.match(tools, /register_extension_tool/);

  const privateResearch = spawnSync(
    "rg",
    ["-n", "clark_research|ClarkResearch|cloud_advisor|CloudAdvisor", "crates/provider-local/src"],
    { cwd: root, encoding: "utf8" },
  );
  assert.equal(privateResearch.status, 1, privateResearch.stdout || privateResearch.stderr);
});

test("cloud context and cartography routes are host-injected", () => {
  const context = readFileSync(
    resolve(root, "crates/provider-local/src/platform.rs"),
    "utf8",
  );
  assert.match(context, /trait PlatformContextProvider/);
  assert.doesNotMatch(
    context,
    /reqwest|\/memories|organization-knowledge|code.*repositories.*context/i,
  );

  const cartography = readFileSync(
    resolve(root, "crates/scout-platform-client/src/lib.rs"),
    "utf8",
  );
  assert.match(cartography, /route_prefix/);
  assert.doesNotMatch(cartography, /\/v1\/system-cartography|ClarkCartography/);
});

test("credential and worker identities are neutral foundation contracts", () => {
  const credentials = readFileSync(
    resolve(root, "src-tauri/src/session_credentials.rs"),
    "utf8",
  );
  assert.doesNotMatch(credentials, /CLKCRD|clark-desktop-credentials/i);

  const worker = readFileSync(resolve(root, "crates/code-worker/Cargo.toml"), "utf8");
  assert.match(worker, /name = "agent-code-worker"/);
  assert.doesNotMatch(worker, /agent-desktop-worker/i);
});

test("specialist runtime policy and product auth stay downstream", () => {
  const specialists = readFileSync(resolve(root, "app/src/lib/specialists.ts"), "utf8");
  assert.match(specialists, /definition\.runtime\.modelRoute/);
  assert.doesNotMatch(specialists, /clark_deepseek|modelRoute:\s*"clark/i);

  const envExample = readFileSync(resolve(root, "app/.env.example"), "utf8");
  assert.doesNotMatch(envExample, /CLARK_|clarkchat|clarkslabs/i);

  const commands = readFileSync(resolve(root, "app/src/lib/slashCommands.ts"), "utf8");
  assert.doesNotMatch(commands, /scout:scout|security:security-(?:scan|diff|deep)|subscriber_workflows/);
  assert.match(commands, /localAgent\.gatedWorkflows/);
});

test("Clark identifiers in foundation source are limited to versioned Scout wire ABI", () => {
  const scan = spawnSync(
    "rg",
    [
      "-n",
      "-i",
      "\\bclark\\b|clark[_-]|\\.clark",
      "app/src",
      "src-tauri/src",
      "crates",
      "--glob",
      "**/src/**",
      "--glob",
      "!**/tests.rs",
      "--glob",
      "!**/tests/**",
    ],
    { cwd: root, encoding: "utf8" },
  );
  assert.equal(scan.status, 0, scan.stderr);
  const lines = scan.stdout.trim().split("\n").filter(Boolean);
  assert.ok(lines.length > 0, "the compatibility allowlist should remain explicit");
  for (const line of lines) {
    const path = line.split(":", 1)[0];
    assert.match(
      path,
      /crates\/(?:scout-|agent-orchestration\/src\/scout\/)/,
      `non-protocol Clark identifier leaked into ${line}`,
    );
    assert.match(
      line,
      /clark(?:\.scout|\.system-cartography|\/github|\/gitlab|\/aws|\/gcp|-scout-key)/i,
      `unrecognized Scout compatibility identifier: ${line}`,
    );
  }
});

test("documentation, fixtures, and configuration contain no downstream runtime policy", () => {
  const files = execFileSync(
    "git",
    ["ls-files", "-co", "--exclude-standard", "-z"],
    { cwd: root, encoding: "utf8" },
  ).split("\0").filter(Boolean);
  const textExtensions = new Set([
    ".cmd", ".css", ".example", ".html", ".json", ".lock", ".md", ".mjs",
    ".nsh", ".plist", ".ps1", ".py", ".rs", ".sh", ".swift", ".toml",
    ".ts", ".tsx", ".txt", ".yaml", ".yml",
  ]);
  const forbidden = /api\.clarkslabs|clarkchat|com\.clark|clark-code|clark_code|clark:\/\/|CLARK_[A-Z]|\.clark(?:\/|\b)|Kimi K3|DeepSeek V4|qwen3\.7|cloud_advisor|ClarkResearch|subscriber_workflows|Subscriber workflow|subscription unlocks this workflow|paid seat|Pro coverage/i;
  const leaks = [];
  for (const relative of files) {
    if (
      relative === "harness/product-boundary.spec.mjs"
      || relative.startsWith("vendor/")
      || !existsSync(resolve(root, relative))
      || !textExtensions.has(relative.slice(relative.lastIndexOf(".")))
    ) continue;
    const text = readFileSync(resolve(root, relative), "utf8");
    if (forbidden.test(text)) leaks.push(relative);
  }
  assert.deepEqual(leaks, []);
});
