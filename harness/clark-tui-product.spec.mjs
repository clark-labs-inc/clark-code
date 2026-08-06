import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import path from "node:path";
import test from "node:test";

import { evaluateContract } from "./clark-tui-product.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");

test("evaluates the Clark-native terminal product contract", async () => {
  const receipt = await evaluateContract();
  assert.equal(receipt.receipt_type, "clark_tui_product_contract");
  assert.equal(receipt.summary.feature_count, 10);
  assert.equal(receipt.summary.implemented_count, 10);
  assert.equal(receipt.summary.gap_count, 0);
  assert.equal(receipt.summary.complete, true);
  assert.equal(receipt.summary.command_count, 8);
  assert.equal(receipt.summary.command_gaps, 0);
  assert.ok(receipt.summary.behavior_count >= 25);

  const minimal = receipt.features.find((feature) => feature.id === "minimal_tui_surface");
  assert.equal(minimal?.state, "implemented");
  const specialists = receipt.features.find((feature) => feature.id === "typed_specialist_workspaces");
  assert.equal(specialists?.state, "implemented");
  const cloud = receipt.features.find((feature) => feature.id === "specialist_cloud_continuity");
  assert.equal(cloud?.state, "implemented");
  const continuity = receipt.features.find(
    (feature) => feature.id === "shared_gui_cli_conversation_continuity",
  );
  assert.equal(continuity?.state, "implemented");
  assert.equal(continuity?.first_failure, null);
  assert.ok(
    receipt.features.every((feature) => feature.state === "implemented" || feature.first_failure),
  );
});

test("the Clark implementation boundary rejects external TUI integration", async () => {
  const receipt = await evaluateContract();
  assert.ok(receipt.implementation_boundary.scanned_file_count > 5);
  assert.equal(receipt.implementation_boundary.rule.includes("original Clark code"), true);
  assert.deepEqual(receipt.implementation_boundary.terminal_crates, ["crossterm", "ratatui"]);
  assert.deepEqual(receipt.implementation_boundary.forbidden_source_terms, ["codex"]);
});

test("the completion gate is green when every Clark product contract is implemented", () => {
  const result = spawnSync(
    process.execPath,
    [path.join(ROOT, "harness", "clark-tui-product.mjs"), "--require-complete"],
    { cwd: ROOT, encoding: "utf8" },
  );
  assert.equal(result.status, 0, result.stderr || result.stdout);
  assert.match(result.stdout, /10\/10 capability groups implemented/);
});
