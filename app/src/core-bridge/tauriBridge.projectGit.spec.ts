import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { TauriBridge } from "./tauriBridge";

describe("TauriBridge remote project Git", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
  });

  it("routes branch and worktree operations through the selected remote executor", async () => {
    const bridge = new TauriBridge();
    const remote = { id: "native-remote-handle" };

    await bridge.listProjectBranches("/srv/project", remote);
    await bridge.switchProjectBranch("/srv/project", "feature/remote", remote);
    await bridge.createPermanentWorktree("/srv/project", "remote-work", remote);

    expect(invoke).toHaveBeenNthCalledWith(1, "project_branch_list", {
      projectPath: "/srv/project",
      remote,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "project_branch_switch", {
      projectPath: "/srv/project",
      branch: "feature/remote",
      remote,
    });
    expect(invoke).toHaveBeenNthCalledWith(3, "project_worktree_create", {
      projectPath: "/srv/project",
      name: "remote-work",
      remote,
    });
  });

  it("opens a newly bound session with one atomic native invocation", async () => {
    const bridge = new TauriBridge();
    const config = { cwd: "/srv/project", extra: { model: "local-model" } };

    await bridge.openSession("local", config, {
      kind: "new",
      options: { cwd: "/srv/project", mode: "auto" },
      bindId: "conversation-1",
    });

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("session_open", {
      providerId: "local",
      config,
      request: {
        kind: "new",
        options: { cwd: "/srv/project", mode: "auto" },
        bind_id: "conversation-1",
      },
    });
  });

  it("loads a session with one atomic native invocation", async () => {
    const bridge = new TauriBridge();

    await bridge.openSession("local", {}, {
      kind: "load",
      id: "conversation-2",
    });

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("session_open", {
      providerId: "local",
      config: {},
      request: { kind: "load", id: "conversation-2" },
    });
  });

  it("does not send a renderer-selected account partition when listing global memory", async () => {
    const bridge = new TauriBridge();

    await bridge.listGlobalMemory();

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("local_list_global_memory");
  });

  it("resumes a provider incident without echoing a synthetic user turn", async () => {
    const bridge = new TauriBridge();

    await bridge.resumeProviderIncident("conversation-3");

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("prompt", {
      sessionId: "conversation-3",
      blocks: [{
        type: "text",
        text: "Continue from the saved progress. Re-read current state, do not repeat completed writes, and finish the task.",
      }],
      attachments: [],
      internalResume: true,
    });
  });
});
