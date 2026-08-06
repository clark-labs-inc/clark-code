import { afterEach, describe, expect, it, vi } from "vitest";

const listen = vi.hoisted(() => vi.fn(async () => () => {}));

vi.mock("@tauri-apps/api/event", () => ({ listen }));

import { onSettingsMenuRequested, onUpdateMenuRequested } from "./nativeMenu";

afterEach(() => {
  listen.mockClear();
  vi.unstubAllGlobals();
});

describe("native menu events", () => {
  it("subscribes Settings and updater actions to their native event names", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    const settingsHandler = vi.fn();
    const updateHandler = vi.fn();

    await onSettingsMenuRequested(settingsHandler);
    await onUpdateMenuRequested(updateHandler);

    expect(listen).toHaveBeenNthCalledWith(1, "settings-menu-requested", settingsHandler);
    expect(listen).toHaveBeenNthCalledWith(2, "update-menu-requested", updateHandler);
  });

  it("is a no-op in the browser preview", async () => {
    await onSettingsMenuRequested(vi.fn());
    expect(listen).not.toHaveBeenCalled();
  });
});
