import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionStore } from "../store/sessionStore";
import { liveSessions } from "../store/sessionStore.runtime";
import { currentProjects, startMobileRemoteSession } from "./mobileRemoteProjects";
import type { CodeRemoteCommand } from "./mobileRemote";
import type { Session } from "../core-bridge/types";
import { saveSshHosts } from "./sshHosts";

const session = (id: string) => ({ id, provider: "local" }) as Session;
const command = (session_target?: object, project_id?: string): CodeRemoteCommand => ({
  command_id: "command", host_id: "computer", command_type: "start_session", project_id,
  request: { session_target }, status: "accepted", created_at: "", updated_at: "",
});
const startNewProject = vi.fn(async () => { useSessionStore.setState({ session: session("new"), error: null }); });
const startQuickChat = vi.fn(async () => { useSessionStore.setState({ session: session("new-project"), error: null }); });
const openConversation = vi.fn(async (id: string) => { useSessionStore.setState({ session: session(id), activeProjectRoot: "/historical/folder", activeRemoteHost: null }); });

beforeEach(() => {
  localStorage.clear();
  liveSessions.clear();
  vi.clearAllMocks();
  useSessionStore.setState({
    auth: null, recentProjects: ["/work/previous", "/work/current"],
    localSettings: { ...useSessionStore.getState().localSettings, cwd: "/work/current" },
    session: null, error: null, startNewProject, startQuickChat, openConversation,
    startSession: vi.fn(async () => { useSessionStore.setState({ session: session("registered-new"), error: null }); }),
    endSession: vi.fn(() => { useSessionStore.setState({ session: null }); }),
  });
});

describe("mobile project and session starts", () => {
  it("publishes previously selected folders alongside the current folder without duplicates", () => {
    expect(currentProjects().map((project) => project.root)).toEqual(["/work/current", "/work/previous"]);
  });
  it("creates a fresh managed workspace without requiring an existing project or prompt", async () => {
    await startMobileRemoteSession(command({ kind: "new_project" }));
    expect(startQuickChat).toHaveBeenCalledOnce();
    expect(useSessionStore.getState().session?.id).toBe("new-project");
  });
  it("resolves historical local projects through the canonical conversation opener", async () => {
    await startMobileRemoteSession(command({ kind: "conversation", conversation_id: "old" }));
    expect(openConversation).toHaveBeenCalledWith("old");
    expect(startNewProject).toHaveBeenCalledWith({ kind: "local", path: "/historical/folder", base: "current" });
    expect(useSessionStore.getState().session?.id).toBe("new");
  });
  it("uses the conversation's exact SSH folder even when the host default changed", async () => {
    saveSshHosts([{ id: "ssh", label: "Server", host: "user@server", remoteRoot: "/default" }], null);
    openConversation.mockImplementationOnce(async (id) => {
      useSessionStore.setState({ session: session(id), activeProjectRoot: "/saved/project", activeRemoteHost: "user@server" });
    });
    await startMobileRemoteSession(command({ kind: "conversation", conversation_id: "remote-old" }));
    expect(startNewProject).toHaveBeenCalledWith({ kind: "remote", host: { id: "ssh", label: "Server", host: "user@server", remoteRoot: "/saved/project" } });
  });
  it("does not substitute another project if the historical conversation cannot open", async () => {
    openConversation.mockImplementationOnce(async () => {});
    await expect(startMobileRemoteSession(command({ kind: "conversation", conversation_id: "missing" })))
      .rejects.toMatchObject({ code: "project_unavailable" });
    expect(startNewProject).not.toHaveBeenCalled();
  });
  it("keeps registered project starts available and creates a separate session", async () => {
    await startMobileRemoteSession(command(undefined, "local:%2Fwork%2Fprevious"));
    expect(useSessionStore.getState().localSettings.cwd).toBe("/work/previous");
    expect(useSessionStore.getState().session?.id).toBe("registered-new");
  });
});
