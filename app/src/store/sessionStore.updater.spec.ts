import { beforeEach, describe, expect, it, vi } from "vitest";

const updater = vi.hoisted(() => ({
  checkAndStageUpdate: vi.fn(),
  refreshStagedUpdate: vi.fn(),
  installStagedUpdate: vi.fn(async () => {}),
  beginUpdateDrain: vi.fn(async () => 0),
  cancelUpdateDrain: vi.fn(async () => {}),
  relaunchApp: vi.fn(async () => {}),
  consumeJustUpdated: vi.fn(async () => null),
}));
const flushCloudPuts = vi.hoisted(() => vi.fn(async () => true));

vi.mock("../lib/updater", () => updater);
vi.mock("../lib/nativeMenu", () => ({
  onSettingsMenuRequested: vi.fn(async () => () => {}),
  onUpdateMenuRequested: vi.fn(async () => () => {}),
}));
vi.mock("../lib/cloudHistory", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/cloudHistory")>()),
  flushCloudPuts,
}));

import { useSessionStore } from "./sessionStore";

beforeEach(() => {
  vi.clearAllMocks();
  updater.installStagedUpdate.mockResolvedValue(undefined);
  updater.refreshStagedUpdate.mockResolvedValue({
    status: "ready",
    update: { version: "0.1.65" },
  });
  updater.beginUpdateDrain.mockResolvedValue(0);
  updater.cancelUpdateDrain.mockResolvedValue(undefined);
  updater.relaunchApp.mockResolvedValue(undefined);
  updater.consumeJustUpdated.mockResolvedValue(null);
  flushCloudPuts.mockResolvedValue(true);
  useSessionStore.getState().endSession({ force: true });
  useSessionStore.setState({
    update: null,
    updateProgress: null,
    updateChecking: false,
    updateWaiting: false,
    updateApplying: false,
    connecting: false,
    error: null,
  });
});

describe("update coordinator", () => {
  it("stages a verified update and exposes it to every update control", async () => {
    updater.checkAndStageUpdate.mockResolvedValue({
      status: "ready",
      update: { version: "0.1.65" },
    });

    const result = await useSessionStore.getState().checkForUpdate();

    expect(result).toEqual({ status: "ready", update: { version: "0.1.65" } });
    expect(useSessionStore.getState()).toMatchObject({
      update: { version: "0.1.65" },
      updateChecking: false,
      updateProgress: null,
    });
  });

  it("keeps update failures distinct from an up-to-date result", async () => {
    updater.checkAndStageUpdate.mockResolvedValue({
      status: "error",
      message: "signature verification failed",
    });

    const result = await useSessionStore.getState().checkForUpdate();

    expect(result).toEqual({ status: "error", message: "signature verification failed" });
    expect(useSessionStore.getState().update).toBeNull();
  });

  it("allows only one manifest/download check at a time", async () => {
    let finish!: (result: { status: "up-to-date" }) => void;
    updater.checkAndStageUpdate.mockImplementation(
      () => new Promise((resolve) => {
        finish = resolve;
      }),
    );

    const first = useSessionStore.getState().checkForUpdate();
    expect(useSessionStore.getState().updateChecking).toBe(true);
    await expect(useSessionStore.getState().checkForUpdate()).resolves.toEqual({ status: "busy" });
    expect(updater.checkAndStageUpdate).toHaveBeenCalledTimes(1);

    finish({ status: "up-to-date" });
    await expect(first).resolves.toEqual({ status: "up-to-date" });
    expect(useSessionStore.getState().updateChecking).toBe(false);
  });

  it("drains, installs, and requests relaunch when the ready action is used", async () => {
    updater.relaunchApp.mockImplementation(() => new Promise(() => {}));
    useSessionStore.setState({ update: { version: "0.1.65" } });

    void useSessionStore.getState().applyUpdate();

    await vi.waitFor(() => expect(updater.relaunchApp).toHaveBeenCalledOnce());
    expect(updater.beginUpdateDrain).toHaveBeenCalledOnce();
    expect(updater.refreshStagedUpdate).toHaveBeenCalledOnce();
    expect(flushCloudPuts).toHaveBeenCalledOnce();
    expect(updater.installStagedUpdate).toHaveBeenCalledOnce();
    expect(useSessionStore.getState()).toMatchObject({
      updateWaiting: false,
      updateApplying: true,
    });
  });

  it("replaces a superseded staged release before installing", async () => {
    updater.refreshStagedUpdate.mockResolvedValue({
      status: "ready",
      update: { version: "0.1.88" },
    });
    updater.relaunchApp.mockImplementation(() => new Promise(() => {}));
    useSessionStore.setState({ update: { version: "0.1.65" } });

    void useSessionStore.getState().applyUpdate();

    await vi.waitFor(() => expect(updater.installStagedUpdate).toHaveBeenCalledOnce());
    expect(updater.refreshStagedUpdate).toHaveBeenCalledBefore(updater.installStagedUpdate);
    expect(useSessionStore.getState().update).toEqual({ version: "0.1.88" });
  });

  it("fails closed when the latest release cannot be confirmed", async () => {
    updater.refreshStagedUpdate.mockResolvedValue({
      status: "error",
      message: "update channel did not stabilize",
    });
    useSessionStore.setState({ update: { version: "0.1.65" } });

    await useSessionStore.getState().applyUpdate();

    expect(updater.installStagedUpdate).not.toHaveBeenCalled();
    expect(updater.cancelUpdateDrain).toHaveBeenCalledOnce();
    expect(useSessionStore.getState()).toMatchObject({
      update: { version: "0.1.65" },
      updateWaiting: false,
      updateApplying: false,
      updateProgress: null,
      error: "Error: update channel did not stabilize",
    });
  });
});
