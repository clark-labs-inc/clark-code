import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Snapshot } from "../core-bridge/types";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import {
  configureArtifactCloudCredentials,
  forgetArtifactCloudConversation,
  resetArtifactCloudSync,
  scheduleArtifactCloudSync,
  snapshotForArtifactCloud,
} from "./cloudArtifacts";

const creds = { endpoint: "https://example.test", token: "token" };

function snapshot(toolCall = "tool-1"): Snapshot {
  return {
    runs: {},
    timeline: [{ item: "artifact", id: "doc:/Users/alice/.clark/workspace/desk-1/report.md" }],
    tool_calls: {},
    artifacts: [{
      id: "doc:/Users/alice/.clark/workspace/desk-1/report.md",
      title: "report.md",
      kind: "file",
      mime_type: "text/markdown",
      uri: "/Users/alice/.clark/workspace/desk-1/report.md",
      tool_call: toolCall,
    }],
    provider_incidents: {},
  };
}

beforeEach(() => {
  resetArtifactCloudSync();
  configureArtifactCloudCredentials(creds);
  invoke.mockReset();
});

describe("mandatory generated artifact cloud sync", () => {
  it("persists a safe retry URI and never serializes the absolute host path", () => {
    const cloud = snapshotForArtifactCloud("desk-1", snapshot());
    expect(cloud.artifacts[0]).toMatchObject({
      id: "doc:report.md",
      uri: "clark-workspace://desk-1/report.md",
    });
    expect(cloud.timeline[0]).toEqual({ item: "artifact", id: "doc:report.md" });
    expect(JSON.stringify(cloud)).not.toContain("/Users/alice");
  });

  it("replaces the retry URI only after native upload completion", async () => {
    invoke.mockResolvedValue({
      artifact_id: "deskart_1",
      logical_id: "doc:report.md",
      filename: "report.md",
      content_type: "text/markdown",
      size_bytes: 12,
      sha256: "a".repeat(64),
      state: "uploaded",
      uri: "/api/desktop/conversations/desk-1/artifacts/deskart_1",
    });
    const onReady = vi.fn();
    const local = snapshot();
    scheduleArtifactCloudSync(creds, "desk-1", local, onReady);
    await vi.waitFor(() => expect(onReady).toHaveBeenCalledOnce());

    expect(invoke).toHaveBeenCalledWith("desktop_artifact_upload", {
      endpoint: creds.endpoint,
      token: creds.token,
      desktopId: "desk-1",
      logicalId: "doc:report.md",
      sourceUri: "/Users/alice/.clark/workspace/desk-1/report.md",
    });
    expect(snapshotForArtifactCloud("desk-1", local).artifacts[0].uri)
      .toBe("/api/desktop/conversations/desk-1/artifacts/deskart_1");
  });

  it("treats a rewrite from a later tool call as a new immutable version", async () => {
    invoke.mockImplementation(async (_command, args: { logicalId: string }) => ({
      artifact_id: `deskart_${invoke.mock.calls.length}`,
      logical_id: args.logicalId,
      filename: "report.md",
      content_type: "text/markdown",
      size_bytes: 12,
      sha256: "b".repeat(64),
      state: "uploaded",
      uri: `/api/desktop/conversations/desk-1/artifacts/deskart_${invoke.mock.calls.length}`,
    }));
    scheduleArtifactCloudSync(creds, "desk-1", snapshot("tool-1"), () => {});
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    scheduleArtifactCloudSync(creds, "desk-1", snapshot("tool-2"), () => {});
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
  });

  it("fences an in-flight completion when the conversation is deleted", async () => {
    let resolveUpload!: (artifact: unknown) => void;
    invoke.mockReturnValue(new Promise((resolve) => {
      resolveUpload = resolve;
    }));
    const onReady = vi.fn();
    const local = snapshot();
    scheduleArtifactCloudSync(creds, "desk-1", local, onReady);
    forgetArtifactCloudConversation("desk-1");
    resolveUpload({
      artifact_id: "deskart_late",
      logical_id: "doc:report.md",
      filename: "report.md",
      content_type: "text/markdown",
      size_bytes: 12,
      sha256: "c".repeat(64),
      state: "uploaded",
      uri: "/api/desktop/conversations/desk-1/artifacts/deskart_late",
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(onReady).not.toHaveBeenCalled();
    expect(snapshotForArtifactCloud("desk-1", local).artifacts[0].uri)
      .toBe("clark-workspace://desk-1/report.md");
  });

  it("surfaces a permanent quota failure without an endless retry loop", async () => {
    invoke.mockRejectedValue(new Error("artifact initiation failed (403 Forbidden): quota exceeded"));
    const warning = vi.fn();
    const local = snapshot();
    scheduleArtifactCloudSync(creds, "desk-1", local, () => {}, warning);
    await vi.waitFor(() => expect(warning).toHaveBeenCalledOnce());

    scheduleArtifactCloudSync(creds, "desk-1", local, () => {}, warning);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(invoke).toHaveBeenCalledTimes(1);
  });
});
