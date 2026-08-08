import { beforeEach, describe, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { remoteWorkerConnect } from "./remoteWorker";

describe("remote worker attachment", () => {
  beforeEach(() => {
    invoke.mockReset();
    vi.useRealTimers();
  });

  it("coalesces concurrent attachment requests for the same boundary", async () => {
    const connection = {
      id: "worker-1",
      cwd: "/repo",
      arch: "linux-x86_64",
      sshTransport: "control_master" as const,
      connectionKind: "started" as const,
      connectDurationMs: 42,
      accountWorkerCount: 1,
    };
    let resolve!: (value: typeof connection) => void;
    invoke.mockReturnValue(new Promise((done) => { resolve = done; }));
    const first = remoteWorkerConnect("host", "/repo", "local-model", "max");
    const second = remoteWorkerConnect("host", "/repo", "local-model", "max");
    expect(invoke).toHaveBeenCalledTimes(1);
    resolve(connection);
    await expect(Promise.all([first, second])).resolves.toEqual([
      connection,
      connection,
    ]);
  });

  it("leaves retry classification to the native registry", async () => {
    invoke.mockRejectedValue("remote worker model is invalid");
    await expect(
      remoteWorkerConnect("host", "/repo", "retired-model", "max"),
    ).rejects.toBe("remote worker model is invalid");
    expect(invoke).toHaveBeenCalledTimes(1);
  });
});
