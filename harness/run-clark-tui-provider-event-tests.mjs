import { spawnSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoDir = path.resolve(harnessDir, "..");
const crateDir = path.join(repoDir, "target", "clark-tui-product", "harness-crate");
const manifest = path.join(crateDir, "Cargo.toml");

function run(program, args, env = process.env) {
  const result = spawnSync(program, args, {
    cwd: repoDir,
    env,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  });
  if (result.status !== 0) {
    throw new Error(`${program} ${args.join(" ")} failed:\n${result.stderr || result.stdout}`);
  }
  return result.stdout;
}

await mkdir(crateDir, { recursive: true });
await writeFile(
  manifest,
  `[package]
name = "clark-tui-contract-tests"
version = "0.0.0"
edition = "2021"
publish = false

[workspace]

[dependencies]
agent-core = { path = "../../../crates/agent-core" }
base64 = "0.22"
directories = "6"
serde = { version = "1", features = ["derive"] }
serde_json = { version = "1", features = ["preserve_order"] }
tempfile = "3"
unicode-width = "0.2"

[lib]
path = "../../../harness/clark-tui-provider-events-test.rs"
`,
);

process.stdout.write(
  run(
    "cargo",
    ["test", "--manifest-path", manifest, "--", "--nocapture"],
    {
      ...process.env,
      CARGO_TARGET_DIR: path.join(crateDir, "target"),
    },
  ),
);
