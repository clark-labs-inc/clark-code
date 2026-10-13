import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emptySnapshot, type Snapshot } from "../core-bridge/types";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
import { cloudGet, configureCloudHistoryCredentials, prepareCloudDurability, resetCloudHistory, scheduleCloudPut } from "./cloudHistory";
const creds = { accountScope: "id:streaming-fixture" };
beforeEach(() => { resetCloudHistory(); configureCloudHistoryCredentials(creds); invoke.mockReset(); });
afterEach(() => resetCloudHistory());

describe("streaming after native history publication", () => {
  it("restores the native sealed-prefix observation when reopening a paged conversation", async () => {
    const snapshot: Snapshot = { ...emptySnapshot(), runs: { current: { id: "current", status: "running" } },
      timeline: Array.from({ length: 400 }, (_, index) => ({ item: "message", role: "agent", run: "current",
        blocks: [{ type: "text", text: `token-${index}` }] })) };
    invoke.mockImplementation(async (command: string) => {
      if (command === "desktop_conv_get") return { id: "reopened", rev: 5, sealedThrough: 240, snapshot };
      if (command === "desktop_conv_commit_pending") return undefined;
      if (command === "desktop_conv_publish") return { outcome: "published", rev: 6, sealedThrough: 240, large: false };
      throw new Error(`unexpected command ${command}`);
    });
    const reopened = await cloudGet(creds, "reopened");
    scheduleCloudPut(creds, { id: "reopened", title: "Reopened", provider: "local", createdAt: 1, updatedAt: 1 }, reopened!, "running");
    await expect(prepareCloudDurability(1000)).resolves.toBe(true);
    expect(invoke.mock.calls.filter(([command]) => command === "desktop_conv_publish")).toHaveLength(1);
  });
  it("uses the native sealed-prefix receipt to publish a live tail of a long conversation", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "desktop_conv_commit_pending") return undefined;
      if (command === "desktop_conv_publish") return { outcome: "published", rev: Date.now(), sealedThrough: 240, large: false };
      throw new Error(`unexpected command ${command}`);
    });
    const snapshot: Snapshot = { ...emptySnapshot(), timeline: Array.from({ length: 400 }, (_, index) => ({
      item: "message", role: "user", run: "previous", blocks: [{ type: "text", text: `turn-${index}` }],
    })), model_context_checkpoint: { timeline_index: 400, transcript: { items: [], truncated: true } } };
    const meta = { id: "paged-chat", title: "Paged chat", provider: "local", createdAt: 1, updatedAt: 1 };
    scheduleCloudPut(creds, meta, snapshot, "idle");
    await expect(prepareCloudDurability(1000)).resolves.toBe(true);
    const live: Snapshot = { ...snapshot, runs: { current: { id: "current", status: "running" } },
      timeline: [...snapshot.timeline, { item: "message", role: "agent", run: "current", blocks: [{ type: "text", text: "First token" }] }] };
    scheduleCloudPut(creds, meta, live, "running");
    await expect(prepareCloudDurability(1000)).resolves.toBe(true);
    expect(invoke.mock.calls.filter(([command]) => command === "desktop_conv_publish")).toHaveLength(2);
    // Native still receives the exact full durable projection; the renderer
    // only uses its receipt to decide that this live head is small enough.
    expect(invoke).toHaveBeenCalledWith("desktop_conv_commit_pending", {
      commit: expect.objectContaining({ snapshot: live, status: "running" }),
    });
  });
});
