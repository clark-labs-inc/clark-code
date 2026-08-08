import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const check = vi.hoisted(() => vi.fn());
const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/plugin-updater", () => ({ check }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

function updateCandidate(version: string, body = "") {
  return {
    version,
    body,
    close: vi.fn(async () => {}),
    download: vi.fn(async (onEvent?: (event: object) => void) => {
      onEvent?.({ event: "Started", data: { contentLength: 10 } });
      onEvent?.({ event: "Progress", data: { chunkLength: 4 } });
      onEvent?.({ event: "Progress", data: { chunkLength: 6 } });
      onEvent?.({ event: "Finished", data: {} });
    }),
    install: vi.fn(async () => {}),
  };
}

beforeEach(() => {
  vi.resetModules();
  check.mockReset();
  invoke.mockReset();
  invoke.mockResolvedValue(true);
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
});

afterEach(() => vi.unstubAllGlobals());

describe("desktop updater", () => {
  it("does not consult the production channel for a development flavor", async () => {
    invoke.mockResolvedValue(false);
    const { checkAndStageUpdate } = await import("./updater");

    await expect(checkAndStageUpdate()).resolves.toEqual({ status: "unavailable" });
    expect(check).not.toHaveBeenCalled();
  });

  it("reports an actual no-update response as up to date", async () => {
    check.mockResolvedValue(null);
    const { checkAndStageUpdate } = await import("./updater");

    await expect(checkAndStageUpdate()).resolves.toEqual({ status: "up-to-date" });
  });

  it("does not misreport updater failures as up to date", async () => {
    check.mockRejectedValue(new Error("manifest unavailable"));
    const { checkAndStageUpdate } = await import("./updater");

    await expect(checkAndStageUpdate()).resolves.toEqual({
      status: "error",
      message: "manifest unavailable",
    });
  });

  it("downloads and exposes a verified candidate as ready", async () => {
    const candidate = updateCandidate("0.1.65", "Bug fixes");
    check.mockResolvedValue(candidate);
    const { checkAndStageUpdate } = await import("./updater");
    const progress = vi.fn();

    await expect(checkAndStageUpdate(progress)).resolves.toEqual({
      status: "ready",
      update: { version: "0.1.65", notes: "Bug fixes" },
    });
    expect(progress).toHaveBeenLastCalledWith({ downloaded: 10, total: 10 });
    expect(candidate.close).not.toHaveBeenCalled();
    expect(check).toHaveBeenCalledWith(
      expect.objectContaining({
        headers: expect.objectContaining({ "Cache-Control": expect.stringContaining("no-cache") }),
      }),
    );
    expect(candidate.download).toHaveBeenCalledWith(expect.any(Function));
  });

  it("jumps directly to the newest release before install", async () => {
    const initiallyStaged = updateCandidate("0.1.65");
    const newest = updateCandidate("0.1.88", "Newest");
    const confirmation = updateCandidate("0.1.88", "Newest");
    check
      .mockResolvedValueOnce(initiallyStaged)
      .mockResolvedValueOnce(newest)
      .mockResolvedValueOnce(confirmation);
    const { checkAndStageUpdate, installStagedUpdate, refreshStagedUpdate } =
      await import("./updater");

    await expect(checkAndStageUpdate()).resolves.toMatchObject({
      status: "ready",
      update: { version: "0.1.65" },
    });
    await expect(refreshStagedUpdate()).resolves.toEqual({
      status: "ready",
      update: { version: "0.1.88", notes: "Newest" },
    });
    await installStagedUpdate();

    expect(newest.download).toHaveBeenCalledOnce();
    expect(initiallyStaged.close).toHaveBeenCalledOnce();
    expect(confirmation.close).toHaveBeenCalledOnce();
    expect(initiallyStaged.install).not.toHaveBeenCalled();
    expect(newest.install).toHaveBeenCalledOnce();
  });

  it("follows multiple releases until the latest pointer stabilizes", async () => {
    const v65 = updateCandidate("0.1.65");
    const v86 = updateCandidate("0.1.86");
    const v88 = updateCandidate("0.1.88");
    const v88Confirmation = updateCandidate("0.1.88");
    check
      .mockResolvedValueOnce(v65)
      .mockResolvedValueOnce(v86)
      .mockResolvedValueOnce(v88)
      .mockResolvedValueOnce(v88Confirmation);
    const { checkAndStageUpdate, refreshStagedUpdate } = await import("./updater");

    await checkAndStageUpdate();
    await expect(refreshStagedUpdate()).resolves.toMatchObject({
      status: "ready",
      update: { version: "0.1.88" },
    });

    expect(v86.download).toHaveBeenCalledOnce();
    expect(v88.download).toHaveBeenCalledOnce();
    expect(v65.close).toHaveBeenCalledOnce();
    expect(v86.close).toHaveBeenCalledOnce();
    expect(v88Confirmation.close).toHaveBeenCalledOnce();
  });

  it("refuses to replace a staged update with an older cached manifest", async () => {
    const staged = updateCandidate("0.1.88");
    check.mockResolvedValueOnce(staged);
    for (let attempt = 0; attempt < 4; attempt += 1) {
      check.mockResolvedValueOnce(updateCandidate("0.1.86"));
    }
    const { checkAndStageUpdate, refreshStagedUpdate } = await import("./updater");

    await checkAndStageUpdate();
    await expect(refreshStagedUpdate()).resolves.toMatchObject({
      status: "error",
      message: expect.stringContaining("older version 0.1.86"),
    });
    expect(staged.close).not.toHaveBeenCalled();
  });
});
