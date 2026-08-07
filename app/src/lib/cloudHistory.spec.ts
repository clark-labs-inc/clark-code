import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Snapshot } from "../core-bridge/types";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import {
  cloudGet,
  cloudDelete,
  configureCloudHistoryCredentials,
  flushCloudPuts,
  MAX_SNAPSHOT_BYTES,
  onCloudHistoryWarning,
  resetCloudHistory,
  scheduleCloudPut,
} from "./cloudHistory";

const creds = { accountScope: "id:account-one" };

beforeEach(() => {
  resetCloudHistory();
  configureCloudHistoryCredentials(creds);
  invoke.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
  resetCloudHistory();
});

describe("cloud history size backstop", () => {
  it("drops a cloud read that finishes after the account partition changes", async () => {
    let finishRead!: (value: unknown) => void;
    invoke.mockImplementationOnce(
      () => new Promise((resolve) => {
        finishRead = resolve;
      }),
    );
    const accountOne = { ...creds, accountScope: "id:account-one" };
    const accountTwo = { accountScope: "id:account-two" };
    resetCloudHistory();
    configureCloudHistoryCredentials(accountOne);

    const reading = cloudGet(accountOne, "old-account-chat");
    resetCloudHistory();
    configureCloudHistoryCredentials(accountTwo);
    finishRead({
      id: "old-account-chat",
      title: "Old account",
      provider: "local",
      createdAt: 1,
      updatedAt: 1,
      rev: 3,
      snapshot: {
        runs: {},
        timeline: [],
        tool_calls: {},
        artifacts: [],
        provider_incidents: {},
      },
    });

    await expect(reading).resolves.toBeNull();
    expect(invoke).toHaveBeenCalledOnce();
  });

  it("migrates legacy planning timeline rows before a native session resume", async () => {
    invoke.mockResolvedValueOnce({
      id: "legacy-plan",
      title: "Legacy plan",
      provider: "local",
      createdAt: 1,
      updatedAt: 1,
      rev: 3,
      snapshot: {
        runs: {},
        timeline: [{
          item: "plan",
          run: "run-1",
          plan: { phases: [{ title: "Inspect", status: "completed" }] },
        }],
        plan: { phases: [{ title: "Inspect", status: "completed" }] },
        tool_calls: {},
        artifacts: [],
      },
    });

    const snapshot = await cloudGet(creds, "legacy-plan");

    expect(snapshot?.timeline).toEqual([{
      item: "execution_checklist",
      run: "run-1",
      checklist: {
        revision: 0,
        steps: [{ title: "Inspect", status: "completed" }],
      },
    }]);
    expect(snapshot?.execution_checklist).toEqual({
      revision: 0,
      steps: [{ title: "Inspect", status: "completed" }],
    });
    expect("plan" in (snapshot as Snapshot & { plan?: unknown })).toBe(false);
  });

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
      creds,
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

  it("does not repeatedly serialize an already-oversized running transcript", async () => {
    const warning = vi.fn();
    const unsubscribe = onCloudHistoryWarning(warning);
    const snapshot: Snapshot = {
      runs: {},
      timeline: [{
        item: "message",
        run: "r1",
        role: "user",
        blocks: [{ type: "text", text: "x".repeat(MAX_SNAPSHOT_BYTES + 1) }],
      }],
      tool_calls: {},
      artifacts: [],
      provider_incidents: {},
    };
    const meta = {
      id: "oversized-running",
      title: "Oversized running",
      provider: "local",
      createdAt: 1,
      updatedAt: 1,
    };

    scheduleCloudPut(creds, meta, snapshot, "running");
    await vi.waitFor(() => expect(warning).toHaveBeenCalledOnce());
    scheduleCloudPut(creds, meta, snapshot, "running");
    await Promise.resolve();

    expect(warning).toHaveBeenCalledOnce();
    expect(invoke).not.toHaveBeenCalled();
    unsubscribe();
  });

  it("publishes a recovered terminal snapshot after native recovery", async () => {
    const snapshot: Snapshot = {
      history_checkpoint: 9,
      runs: {
        "run-1": {
          id: "run-1",
          status: "failed",
          outcome: { status: "failed", failure_kind: "runtime_interrupted" },
        },
      },
      timeline: [],
      tool_calls: {},
      artifacts: [],
      provider_incidents: {},
      goal: {
        id: "goal-1",
        objective: "finish the work",
        status: "blocked",
        run: "run-1",
        tokens_used: 0,
        time_used_seconds: 0,
        continuations: 0,
        updated_at_ms: 9,
        blocker_reason: "Clark restarted before the goal finished.",
      },
    };
    invoke
      .mockResolvedValueOnce({
        id: "recovered-terminal",
        title: "Recovered terminal run",
        provider: "local",
        createdAt: 1,
        updatedAt: 1,
        rev: 7,
        status: "running",
        snapshotRecoveryRequired: true,
        snapshot,
      })
      .mockResolvedValueOnce({ rev: 8 });

    await expect(cloudGet(creds, "recovered-terminal")).resolves.toMatchObject({
      runs: { "run-1": { status: "failed" } },
      goal: { status: "blocked" },
    });
    await expect.poll(() => invoke.mock.calls.length).toBe(2);

    expect(invoke).toHaveBeenNthCalledWith(1, "desktop_conv_get", {
      id: "recovered-terminal",
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "desktop_conv_put", expect.objectContaining({
      id: "recovered-terminal",
      status: "idle",
      baseRev: 7,
      snapshot: expect.objectContaining({
        runs: expect.objectContaining({
          "run-1": expect.objectContaining({ status: "failed" }),
        }),
        goal: expect.objectContaining({ status: "blocked" }),
      }),
    }));
  });

  it("permanently republishes a sanitized provider-contaminated snapshot", async () => {
    invoke
      .mockResolvedValueOnce({
        id: "provider-residue",
        title: "Provider residue",
        provider: "local",
        createdAt: 1,
        updatedAt: 1,
        rev: 4,
        snapshot: {
          runs: {},
          timeline: [{
            item: "message",
            run: "run-1",
            role: "agent",
            blocks: [{ type: "text", text: "prefix <|begin__of__sentence|> residue" }],
          }],
          model_context_checkpoint: {
            timeline_index: 1,
            transcript: { truncated: false, items: [] },
          },
          tool_calls: {},
          artifacts: [],
          provider_incidents: {},
        },
      })
      .mockResolvedValueOnce({ rev: 5 });

    await expect(cloudGet(creds, "provider-residue")).resolves.toMatchObject({
      timeline: [],
    });
    await expect.poll(() => invoke.mock.calls.length).toBe(2);

    expect(invoke).toHaveBeenNthCalledWith(2, "desktop_conv_put", expect.objectContaining({
      id: "provider-residue",
      baseRev: 4,
      snapshot: expect.objectContaining({ timeline: [] }),
    }));
    expect(invoke.mock.calls[1]?.[1]?.snapshot.model_context_checkpoint).toBeUndefined();
  });

  it("retries a queued snapshot without exposing the refreshed native credential", async () => {
    vi.useFakeTimers();
    invoke
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce({ rev: 8 });
    scheduleCloudPut(creds, {
      id: "refresh-retry",
      title: "Refresh retry",
      provider: "local",
      createdAt: 1,
      updatedAt: 1,
    }, { runs: {}, timeline: [], tool_calls: {}, artifacts: [], provider_incidents: {} });

    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    configureCloudHistoryCredentials(creds);
    await vi.advanceTimersByTimeAsync(1_000);

    expect(invoke).toHaveBeenLastCalledWith("desktop_conv_put", expect.not.objectContaining({
      token: expect.anything(),
      endpoint: expect.anything(),
    }));
  });

  it("waits for an in-flight PUT before deleting and clears its write revision", async () => {
    let resolvePut: (value: { rev: number }) => void = () => {
      throw new Error("PUT resolver was not installed");
    };
    invoke.mockImplementation((command: string) => {
      if (command === "desktop_conv_put") {
        return new Promise<{ rev: number }>((resolve) => { resolvePut = resolve; });
      }
      if (command === "desktop_conv_delete") return Promise.resolve();
      throw new Error(`unexpected command ${command}`);
    });
    scheduleCloudPut(creds, {
      id: "delete-race",
      title: "Delete race",
      provider: "local",
      createdAt: 1,
      updatedAt: 1,
    }, { runs: {}, timeline: [], tool_calls: {}, artifacts: [], provider_incidents: {} });

    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    const deleting = cloudDelete(creds, "delete-race");
    await Promise.resolve();
    expect(invoke).toHaveBeenCalledTimes(1);
    resolvePut({ rev: 8 });
    await deleting;

    expect(invoke).toHaveBeenNthCalledWith(2, "desktop_conv_delete", {
      id: "delete-race",
    });
  });

  it("does not retry a snapshot after another device deleted its conversation", async () => {
    vi.useFakeTimers();
    const warning = vi.fn();
    const unsubscribe = onCloudHistoryWarning(warning);
    invoke.mockRejectedValueOnce(new Error("cloud_deleted: conversation removed"));
    scheduleCloudPut(creds, {
      id: "deleted-elsewhere",
      title: "Deleted elsewhere",
      provider: "local",
      createdAt: 1,
      updatedAt: 1,
    }, { runs: {}, timeline: [], tool_calls: {}, artifacts: [], provider_incidents: {} });

    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    await vi.advanceTimersByTimeAsync(60_000);

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(warning).toHaveBeenCalledWith(expect.stringContaining("deleted on another device"));
    unsubscribe();
  });
});
