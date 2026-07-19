import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const check = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/plugin-updater", () => ({ check }));

import { checkAndStageUpdate } from "./updater";

beforeEach(() => {
  check.mockReset();
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
});

afterEach(() => vi.unstubAllGlobals());

describe("checkAndStageUpdate", () => {
  it("reports an actual no-update response as up to date", async () => {
    check.mockResolvedValue(null);
    await expect(checkAndStageUpdate()).resolves.toEqual({ status: "up-to-date" });
  });

  it("does not misreport updater failures as up to date", async () => {
    check.mockRejectedValue(new Error("manifest unavailable"));
    await expect(checkAndStageUpdate()).resolves.toEqual({
      status: "error",
      message: "manifest unavailable",
    });
  });

  it("downloads and exposes a verified candidate as ready", async () => {
    const candidate = {
      version: "0.1.65",
      body: "Bug fixes",
      close: vi.fn(async () => {}),
      download: vi.fn(async (onEvent: (event: object) => void) => {
        onEvent({ event: "Started", data: { contentLength: 10 } });
        onEvent({ event: "Progress", data: { chunkLength: 4 } });
        onEvent({ event: "Progress", data: { chunkLength: 6 } });
        onEvent({ event: "Finished", data: {} });
      }),
    };
    check.mockResolvedValue(candidate);
    const progress = vi.fn();

    await expect(checkAndStageUpdate(progress)).resolves.toEqual({
      status: "ready",
      update: { version: "0.1.65", notes: "Bug fixes" },
    });
    expect(progress).toHaveBeenLastCalledWith({ downloaded: 10, total: 10 });
    expect(candidate.close).not.toHaveBeenCalled();
  });
});
