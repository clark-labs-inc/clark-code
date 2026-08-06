import path from "node:path";

import { QmpClient } from "./utm-qmp.mjs";

export function selectExpectedRemovableMedia(blocks, expectedBasenames) {
  if (!Array.isArray(blocks) || !Array.isArray(expectedBasenames)) {
    throw new Error("QMP block inventory and expected media must be arrays");
  }
  const expected = new Set(expectedBasenames);
  if (
    expected.size !== expectedBasenames.length
    || [...expected].some((basename) => (
      typeof basename !== "string"
      || !basename
      || path.basename(basename) !== basename
    ))
  ) {
    throw new Error("expected installer media must be unique safe basenames");
  }
  const selected = blocks.filter((block) => (
    block?.removable === true
    && typeof block?.device === "string"
    && typeof block?.inserted?.file === "string"
    && expected.has(path.basename(block.inserted.file))
  ));
  const observed = new Set(selected.map((block) => path.basename(block.inserted.file)));
  const missing = [...expected].filter((basename) => !observed.has(basename));
  if (missing.length > 0) {
    throw new Error(`QMP did not expose expected installer media: ${missing.join(", ")}`);
  }
  return selected.map((block) => ({
    device: block.device,
    basename: path.basename(block.inserted.file),
  }));
}

export async function ejectInstallerMediaAndReset({ port, expectedBasenames }) {
  const client = new QmpClient({ port, timeoutMs: 10_000 });
  try {
    await client.connect();
    const selected = selectExpectedRemovableMedia(
      await client.execute("query-block"),
      expectedBasenames,
    );
    for (const medium of selected) {
      await client.execute("eject", { device: medium.device, force: true });
    }
    const after = await client.execute("query-block");
    const retained = new Map(after.map((block) => [block.device, block.inserted?.file ?? null]));
    const failed = selected.filter((medium) => retained.get(medium.device) !== null);
    if (failed.length > 0) {
      throw new Error(
        `QMP retained installer media in: ${failed.map((medium) => medium.device).join(", ")}`,
      );
    }
    await client.execute("system_reset");
    return {
      transport: "localhost_qmp",
      ejected: selected.map((medium) => medium.basename),
      reset: true,
    };
  } finally {
    client.close();
  }
}
