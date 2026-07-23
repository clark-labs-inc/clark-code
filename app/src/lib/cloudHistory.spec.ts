import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Snapshot } from "../core-bridge/types";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import {
  flushCloudPuts,
  MAX_SNAPSHOT_BYTES,
  onCloudHistoryWarning,
  scheduleCloudPut,
} from "./cloudHistory";

beforeEach(() => invoke.mockReset());

describe("cloud history size backstop", () => {
  it("keeps an oversized UTF-8 snapshot pending and surfaces the failure", async () => {
    const warning = vi.fn();
    const unsubscribe = onCloudHistoryWarning(warning);
    const snapshot: Snapshot = {
      runs: {},
      timeline: [{
        item: "message",
        run: "r1",
        role: "user",
        blocks: [{ type: "text", text: "😀".repeat(Math.ceil(MAX_SNAPSHOT_BYTES / 4)) }],
      }],
      tool_calls: {},
      artifacts: [],
      provider_incidents: {},
    };

    scheduleCloudPut(
      { endpoint: "https://example.test", token: "token", ownerScope: "owner" },
      {
        id: "oversized-utf8",
        title: "Oversized",
        provider: "local",
        createdAt: 1,
        updatedAt: 1,
      },
      snapshot,
    );

    await expect(flushCloudPuts(100)).resolves.toBe(false);
    expect(invoke).not.toHaveBeenCalled();
    expect(warning).toHaveBeenCalledWith(expect.stringContaining("too large to sync safely"));
    unsubscribe();
  });
});
