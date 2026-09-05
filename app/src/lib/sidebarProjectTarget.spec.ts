import { describe, expect, it } from "vitest";
import { sidebarProjectHost } from "./sidebarProjectTarget";
import { sidebarConversationSearchText, type ProjectGroup } from "./projectSidebar";

const group: ProjectGroup = { key: "remote", kind: "remote", label: "API", title: "API", remoteHost: "dev@cpu", remoteRoot: "/work/api", convos: [], latest: 0 };
const host = { id: "cpu", label: "CPU", host: "dev@cpu", remoteRoot: "/work/other" };

describe("sidebar project actions", () => {
  it("uses the row's folder rather than the host's changed default", () => {
    expect(sidebarProjectHost(group, [host])).toEqual({ ...host, remoteRoot: "/work/api" });
    expect(host.remoteRoot).toBe("/work/other");
  });
  it("prefers an exact saved destination and never chooses another host", () => {
    const exact = { ...host, id: "exact", remoteRoot: "/work/api" };
    expect(sidebarProjectHost(group, [host, exact])?.id).toBe("exact");
    expect(sidebarProjectHost(group, [{ ...host, host: "dev@gpu" }])).toBeNull();
  });
  it("finds quick chats and aliases using their visible sidebar labels", () => {
    const id = "00000000-0000-4000-8000-000000000001";
    const conversation = { id, title: "Explain recursion", provider: "local", project: `/mock/.agent/workspace/${id}`, createdAt: 1, updatedAt: 1 };
    expect(sidebarConversationSearchText(conversation, { pinned: [], aliases: {} })).toContain("Quick chats");
    expect(sidebarConversationSearchText({ ...conversation, project: "/work/api" }, { pinned: [], aliases: { "p:/work/api": "Backend" } })).toContain("Backend");
  });
});
