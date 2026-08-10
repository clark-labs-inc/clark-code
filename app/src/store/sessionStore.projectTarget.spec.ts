import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CoreBridge } from "../core-bridge/bridge";
import { emptySnapshot, type Session } from "../core-bridge/types";
import { loadSshHosts, saveSshHosts } from "../lib/sshHosts";
import { useSessionStore } from "./sessionStore";
import { liveSessions, newLiveEntry } from "./sessionStore.runtime";

const bridge = {
  prompt: vi.fn(async () => ({ runId: "run" })),
  cancel: vi.fn(async () => {}),
  respond: vi.fn(async () => {}),
  subscribe: () => () => {},
} as unknown as CoreBridge;

const localSession = {
  id: "local-datasets",
  provider: "local",
} as unknown as Session;

beforeEach(() => {
  localStorage.clear();
  liveSessions.clear();
  useSessionStore.setState({
    bridge,
    providers: [],
    session: null,
    snapshot: emptySnapshot(),
    activeProvider: "local",
    auth: null,
    projectMode: "local",
    selectedHostId: null,
    localSettings: {
      ...useSessionStore.getState().localSettings,
      cwd: "/previous/project",
    },
    activeRemote: null,
    activeRemoteHost: null,
    activeProjectRoot: null,
    connecting: false,
    opening: null,
    conversations: [],
  });
});

describe("new-session target follows the opened conversation", () => {
  it("replaces a sticky SSH target when a local conversation is opened", async () => {
    saveSshHosts([{
      id: "nucleus",
      label: "Nucleus",
      host: "ubuntu@nucleus",
      remoteRoot: "/home/ubuntu/other",
    }], null);
    liveSessions.set(localSession.id, newLiveEntry(localSession, {
      historyPrefix: null,
      remote: null,
      remoteHost: null,
      projectRoot: "/Users/stan/Documents/git/datasets",
    }));
    useSessionStore.setState({
      projectMode: "remote",
      selectedHostId: "nucleus",
      conversations: [{
        id: localSession.id,
        title: "Local datasets",
        provider: "local",
        project: "/Users/stan/Documents/git/datasets",
        createdAt: 1,
        updatedAt: 1,
      }],
    });

    await useSessionStore.getState().openConversation(localSession.id);

    const state = useSessionStore.getState();
    expect(state.projectMode).toBe("local");
    expect(state.localSettings.cwd).toBe("/Users/stan/Documents/git/datasets");
    expect(state.activeRemoteHost).toBeNull();
    expect(state.activeProjectRoot).toBe("/Users/stan/Documents/git/datasets");

    state.endSession();
    expect(useSessionStore.getState()).toMatchObject({
      projectMode: "local",
      localSettings: { cwd: "/Users/stan/Documents/git/datasets" },
    });
  });

  it("pins a reopened remote conversation to its saved host and project root", async () => {
    saveSshHosts([{
      id: "nucleus",
      label: "Nucleus",
      host: "ubuntu@nucleus",
      remoteRoot: "/home/ubuntu/old-project",
    }], null);
    liveSessions.set(localSession.id, newLiveEntry(localSession, {
      historyPrefix: null,
      remote: null,
      remoteHost: "ubuntu@nucleus",
      projectRoot: "/home/ubuntu/datasets",
    }));
    useSessionStore.setState({
      conversations: [{
        id: localSession.id,
        title: "Remote datasets",
        provider: "local",
        project: "/home/ubuntu/datasets",
        remoteHost: "ubuntu@nucleus",
        createdAt: 1,
        updatedAt: 1,
      }],
    });

    await useSessionStore.getState().openConversation(localSession.id);

    expect(useSessionStore.getState()).toMatchObject({
      projectMode: "remote",
      selectedHostId: "nucleus",
      activeRemoteHost: "ubuntu@nucleus",
      activeProjectRoot: "/home/ubuntu/datasets",
    });
    expect(loadSshHosts(null)[0]?.remoteRoot).toBe("/home/ubuntu/datasets");
  });
});
