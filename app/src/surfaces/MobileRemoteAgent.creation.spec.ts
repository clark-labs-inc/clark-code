import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CodeRemoteCommand } from "../lib/mobileRemote";
import type { Session } from "../core-bridge/types";
import { useSessionStore } from "../store/sessionStore";
import { runCommand } from "./MobileRemoteAgent";

const { ack, start, send } = vi.hoisted(() => ({ ack: vi.fn(), start: vi.fn(), send: vi.fn() }));
vi.mock("../lib/mobileRemote", async (original) => ({
  ...await original<typeof import("../lib/mobileRemote")>(), ackCodeRemoteCommand: ack,
}));
vi.mock("../lib/mobileRemoteProjects", () => ({
  currentProjects: () => [], applyProject: vi.fn(), startMobileRemoteSession: start,
}));
vi.mock("../lib/notify", () => ({ notify: vi.fn() }));

const command: CodeRemoteCommand = {
  command_id: "create", host_id: "host", desktop_id: null, command_type: "start_session",
  request: { session_target: { kind: "new_project" }, attachments: [] },
  status: "delivered", created_at: "", updated_at: "",
};
beforeEach(() => {
  vi.clearAllMocks();
  ack.mockImplementation(async (...args) => ({ command: { ...command, status: args[4] } }));
  start.mockImplementation(async () => {
    useSessionStore.setState({ session: { id: "created-session", provider: "local" } as Session });
  });
  useSessionStore.setState({ session: null, send });
});

describe("mobile empty-session receipt", () => {
  it("publishes the new project before completing and returns its new conversation without a model call", async () => {
    const publish = vi.fn(async () => {
      expect(ack).toHaveBeenCalledTimes(1);
      expect(useSessionStore.getState().session?.id).toBe("created-session");
    });
    await runCommand({ accountScope: "owner" }, "host", "instance", command, () => true, publish);
    expect(start).toHaveBeenCalledWith(command);
    expect(send).not.toHaveBeenCalled();
    expect(publish).toHaveBeenCalledOnce();
    expect(ack).toHaveBeenLastCalledWith(
      { accountScope: "owner" }, "host", "instance", "create", "completed",
      expect.objectContaining({ desktop_id: "created-session" }),
    );
    expect(ack.mock.calls[1][5]).not.toHaveProperty("run_id");
  });
  it("does not create a session when the service declines the command claim", async () => {
    ack.mockResolvedValueOnce({ command: { ...command, status: "failed" } });
    const publish = vi.fn(async () => {});
    await runCommand({ accountScope: "owner" }, "host", "instance", command, () => true, publish);
    expect(start).not.toHaveBeenCalled();
    expect(publish).not.toHaveBeenCalled();
  });
});
