import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot } from "../core-bridge/types";
import { DEFAULT_LOCAL_SETTINGS } from "../lib/localAgent";
import { useSessionStore } from "./sessionStore";
import { useSpecialistStore } from "./specialistStore";

describe("Scout conversation start failures", () => {
  beforeEach(() => {
    useSpecialistStore.setState({
      active: "scout",
      contexts: { scout: { kind: "scout" } },
    });
    useSessionStore.setState({
      bridge: {} as CoreBridge,
      auth: null,
      session: null,
      snapshot: emptySnapshot(),
      activeProvider: "local",
      providers: [],
      connecting: false,
      opening: null,
      conversations: [],
      error: null,
      approvalPolicy: "ask",
      collaborationMode: "default",
      localSettings: { ...DEFAULT_LOCAL_SETTINGS, cwd: "/repo" },
    });
  });

  afterEach(() => {
    useSpecialistStore.setState({ active: null, contexts: {} });
  });

  it("surfaces a missing organization without rejecting the composer submit", async () => {
    await expect(useSessionStore.getState().startSession()).resolves.toBeUndefined();

    expect(useSessionStore.getState()).toMatchObject({
      session: null,
      connecting: false,
      error: "Pick or create a Scout workspace before starting.",
    });
  });

  it("does not let Shift+Tab change Scout's fixed Full access policy", () => {
    useSessionStore.getState().cycleApprovalPolicy();

    expect(useSessionStore.getState().approvalPolicy).toBe("ask");
  });

  it("does not let Scout enter read-only Plan mode", () => {
    useSessionStore.getState().setCollaborationMode("plan");

    expect(useSessionStore.getState().collaborationMode).toBe("default");
  });
});
