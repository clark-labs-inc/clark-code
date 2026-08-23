import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function trackedFiles(pathspecs) {
  return execFileSync("git", ["ls-files", "-co", "--exclude-standard", "-z", "--", ...pathspecs], {
    cwd: root,
    encoding: "utf8",
  }).split("\0").filter(Boolean);
}

function matchingLines(pattern, pathspecs) {
  const matches = [];
  for (const relative of trackedFiles(pathspecs)) {
    if (!existsSync(resolve(root, relative))) continue;
    const lines = readFileSync(resolve(root, relative), "utf8").split(/\r?\n/);
    for (const [index, line] of lines.entries()) {
      if (pattern.test(line)) matches.push(`${relative}:${index + 1}:${line}`);
    }
  }
  return matches;
}

test("workspace source dependencies stay inside the repository", () => {
  const metadata = JSON.parse(execFileSync(
    "cargo",
    ["metadata", "--no-deps", "--format-version", "1"],
    { cwd: root, encoding: "utf8" },
  ));
  for (const pkg of metadata.packages) {
    assert.ok(pkg.manifest_path.startsWith(`${root}/`), pkg.manifest_path);
    for (const dependency of pkg.dependencies.filter(({ path }) => path)) {
      assert.ok(dependency.path.startsWith(`${root}/`), `${pkg.name}: ${dependency.path}`);
    }
  }
});

test("the development bundle is Clark Code without release authority", () => {
  const config = JSON.parse(readFileSync(resolve(root, "src-tauri/tauri.conf.json"), "utf8"));
  assert.equal(config.productName, "Clark Code Dev");
  assert.equal(config.app.windows[0].title, "Clark Code Dev");
  assert.equal(config.bundle.createUpdaterArtifacts, false);
  assert.equal(config.plugins?.updater, undefined);

  for (const relative of [
    "src-tauri/tauri.release.conf.json",
    "src-tauri/tauri.windows-signing.conf.json",
    "src-tauri/sign-windows-artifact.ps1",
  ]) {
    assert.equal(spawnSync("test", ["-e", relative], { cwd: root }).status, 1);
  }
});

test("the default product module is locally usable", () => {
  const source = readFileSync(resolve(root, "app/src/product/productModule.ts"), "utf8");
  assert.match(source, /id: "clark_code"/);
  assert.match(source, /name: "Clark Code"/);
  assert.match(source, /authRequired: false/);
  assert.match(source, /defaultModel: "local-model"/);
});

test("downstream product entries share the renderer React runtime", () => {
  const config = readFileSync(resolve(root, "app/vite.config.ts"), "utf8");
  assert.match(config, /dedupe:\s*\["react",\s*"react-dom"\]/);
});

test("account operations cross one opaque renderer boundary", () => {
  const auth = readFileSync(resolve(root, "app/src/lib/auth.ts"), "utf8");
  assert.match(auth, /productRequest<.*>\("account\.(load|sign_in|refresh)"\)/);
  assert.doesNotMatch(auth, /fetch\(|reqwest/i);
});

test("specialist presentation is catalog-driven", () => {
  const specialists = readFileSync(resolve(root, "app/src/lib/specialists.ts"), "utf8");
  const navigation = readFileSync(
    resolve(root, "app/src/surfaces/specialists/SpecialistNavigation.tsx"),
    "utf8",
  );
  assert.match(specialists, /definition\.runtime\.modelRoute/);
  assert.match(navigation, /specialistIcons/);
  assert.doesNotMatch(navigation, /specialistBadge|\.badge/);
  assert.doesNotMatch(`${specialists}\n${navigation}`, /FIRST_PARTY_SPECIALIST_CATALOG|\bPro\b/);
});

test("extension contracts remain dependency-inverted", () => {
  const integration = readFileSync(resolve(root, "src-tauri/src/product.rs"), "utf8");
  assert.match(integration, /trait ProductIntegration/);
  assert.match(integration, /make_provider/);
  assert.match(integration, /prepare_provider_config/);
  assert.match(integration, /async fn request/);

  const tools = readFileSync(resolve(root, "crates/provider-local/src/tools/mod.rs"), "utf8");
  assert.match(tools, /trait ToolPack/);
  assert.match(tools, /register_extension_tool/);
});

test("remote execution policy is prepared by the native integration", () => {
  const source = readFileSync(resolve(root, "src-tauri/src/commands/remote_worker.rs"), "utf8");
  assert.match(source, /prepare_remote_worker/);
  assert.doesNotMatch(source, /API_KEY\s*=|bearer_auth|https?:\/\//i);
});

test("platform context is host-injected", () => {
  const source = readFileSync(resolve(root, "crates/provider-local/src/platform.rs"), "utf8");
  assert.match(source, /trait PlatformContextProvider/);
  assert.doesNotMatch(source, /reqwest|bearer_auth|https?:\/\//i);
});

test("the design system has one token vocabulary", () => {
  const css = readFileSync(resolve(root, "app/src/index.css"), "utf8");
  assert.doesNotMatch(
    css,
    /--color-(?:bg-primary|text-primary|text-secondary|text-muted|text-faint|accent-muted|canvas|surface|primary|focus|on-primary)\s*:/,
  );
  assert.doesNotMatch(css, /Compatibility aliases|Transitional bridge/);
  assert.deepEqual(
    matchingLines(
      /text-\[(?:10px|11px|0\.6875rem)\]|bg-bg-primary|bg-accent.*text-white/,
      ["app/src"],
    ),
    [],
  );
});

test("legacy Scout wire identifiers stay in protocol crates", () => {
  const lines = matchingLines(
    /clark(?:\.scout|\.system-cartography|\/github|\/gitlab|\/aws|\/gcp|-scout-key)/i,
    ["crates"],
  );
  assert.ok(lines.length > 0);
  for (const line of lines) {
    assert.match(
      line,
      /crates\/(?:scout-|agent-orchestration\/src\/scout\/|provider-local\/examples\/scout_benchmark\/)/,
    );
  }
});

test("the README is a clean Clark Code introduction", () => {
  const readme = readFileSync(resolve(root, "README.md"), "utf8");
  assert.match(readme, /^# Clark Code$/m);
  assert.match(readme, /docs\/assets\/clark-code\.png/);
  assert.equal(existsSync(resolve(root, "docs/assets/clark-code.png")), true);
  assert.equal(readme.toLowerCase().includes(["agent", "desktop"].join(" ")), false);
  assert.doesNotMatch(readme, /downstream product|proprietary composition|neutral foundation/i);
});

test("tracked text contains no obvious credential material", () => {
  const textFiles = trackedFiles([
    "README.md", "CONTRIBUTING.md", "docs", "app/src", "src-tauri/src", "crates",
  ]).filter((relative) => (
    /\.(?:md|json|rs|ts|tsx|toml|yml|yaml)$/.test(relative)
      && !relative.includes("/tests/")
      && !relative.endsWith("/tests.rs")
      && !relative.endsWith("_tests.rs")
  ));
  const secret = /-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----|\bAKIA[0-9A-Z]{16}\b|\bxoxb-[A-Za-z0-9-]{20,}\b|\bsk-[A-Za-z0-9]{32,}\b/;
  const leaks = textFiles.filter((relative) => (
    existsSync(resolve(root, relative))
      && secret.test(readFileSync(resolve(root, relative), "utf8"))
  ));
  assert.deepEqual(leaks, []);
});

test("performance instrumentation stays out of an ordinary frontend build", () => {
  // The recorder in app/src/perf exists to measure an optimized build inside
  // the real WebView, so it cannot be gated on `import.meta.env.DEV` — that
  // flag is false in exactly the build we need to observe. Two independent
  // mechanisms keep it out of a normal bundle (a `__CLARK_PERF__` define that
  // is a literal `false`, and an alias that resolves to an empty module). This
  // asserts the outcome rather than either mechanism, so the boundary survives
  // a change to how it is implemented.
  // CI pins pnpm before this contract runs. Invoke that pinned binary directly;
  // `corepack pnpm@10 ...` is not a stable command form across Node releases
  // and can exit 1 without forwarding the underlying build diagnostics.
  const build = spawnSync("pnpm", ["--dir", "app", "build"], {
    cwd: root,
    encoding: "utf8",
    // A cold Vite build on a loaded machine can exceed the default.
    timeout: 600_000,
    env: { ...process.env, VITE_PERF_HOOKS: "" },
  });
  assert.equal(build.status, 0, build.stderr ?? "frontend build failed");

  // app/dist is gitignored, so walk it directly rather than through git.
  const dist = resolve(root, "app/dist");
  assert.ok(existsSync(dist), "app/dist was not produced");
  const emitted = readdirSync(dist, { recursive: true, encoding: "utf8" })
    .filter((name) => name.endsWith(".js"));
  assert.ok(emitted.length > 0, "no JavaScript was emitted to app/dist");

  const forbidden = /__clarkPerf|installPerfHooks|perf_write_report|perf-emit-tick|perf_clock_probe/;
  const leaked = emitted.filter((relative) =>
    forbidden.test(readFileSync(resolve(dist, relative), "utf8")));
  assert.deepEqual(leaked, [], `performance hooks reached a normal build: ${leaked.join(", ")}`);
});
