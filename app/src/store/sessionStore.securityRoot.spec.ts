import { beforeEach, describe, expect, it, vi } from "vitest";

import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot } from "../core-bridge/types";
import { DEFAULT_LOCAL_SETTINGS } from "../lib/localAgent";
import { useSessionStore } from "./sessionStore";
import { useSpecialistStore } from "./specialistStore";

describe("Security task scope", () => {
  beforeEach(() => {
    localStorage.clear();
    useSpecialistStore.setState({
      active: "security",
      contexts: {
        security: { kind: "security", workflow: "security:assistant" },
      },
    });
    useSessionStore.setState({
      bridge: null,
      session: null,
      snapshot: emptySnapshot(),
      activeProvider: "local",
      providers: [],
      auth: null,
      connecting: false,
      opening: null,
      conversations: [],
      localSettings: { ...DEFAULT_LOCAL_SETTINGS, cwd: "/home/engineer" },
      projectMode: "local",
      selectedHostId: null,
      activeRemote: null,
      activeRemoteHost: null,
      activeProjectRoot: null,
      error: null,
    });
  });

  it("launches a Security conversation from a non-repository folder", async () => {
    const openSession = vi.fn(async () => ({ id: "security-task", title: "Security", modes: [], capabilities: {} }));
    const bridge = {
      projectContext: vi.fn(async () => null),
      openSession,
      subscribe: () => () => {},
    } as unknown as CoreBridge;
    useSessionStore.setState({
      bridge,
      providers: [{
        id: "local",
        label: "Clark Code",
        internal: false,
        capabilities: {
          streaming: true,
          permissions: true,
          fs: true,
          terminal: true,
          load_session: false,
          modes: [],
          collaboration_modes: [],
          attachment_kinds: [],
        },
      }],
    });

    await useSessionStore.getState().startSession();

    expect(openSession).toHaveBeenCalledOnce();
    expect(useSessionStore.getState().session?.id).toBe("security-task");
    expect(useSessionStore.getState().error).toBeNull();
  });
});
