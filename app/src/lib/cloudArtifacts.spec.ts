import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Snapshot } from "../core-bridge/types";
import { installProductModule, neutralProduct } from "../product/productModule";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import {
  configureArtifactCloudCredentials,
  forgetArtifactCloudConversation,
  prepareArtifactCloudDurability,
  resetArtifactCloudSync,
  scheduleArtifactCloudSync,
  snapshotForArtifactCloud,
} from "./cloudArtifacts";

const creds = { accountScope: "id:account-one" };

function mockUpload(
  upload: (args: { operation: string; payload: Record<string, unknown> }) => unknown,
): void {
  invoke.mockImplementation(async (command, args) => {
    if (command === "desktop_artifact_stage") {
      return {
        sourceUri: "workspace-artifact://desk-1/.clark-sync/staged-report.md",
        sha256: "d".repeat(64),
        remoteUri: null,
      };
    }
    if (command === "desktop_artifact_mark_uploaded") return undefined;
    if (command === "product_request") return upload(args);
    throw new Error(`unexpected command ${command}`);
  });
}

function productCalls(): unknown[][] {
  return invoke.mock.calls.filter(([command]) => command === "product_request");
}

function snapshot(toolCall = "tool-1"): Snapshot {
  return {
    runs: {},
    timeline: [{ item: "artifact", id: "doc:/Users/alice/.agent/workspace/desk-1/report.md" }],
    tool_calls: {},
    artifacts: [{
      id: "doc:/Users/alice/.agent/workspace/desk-1/report.md",
      title: "report.md",
      kind: "file",
      mime_type: "text/markdown",
      uri: "/Users/alice/.agent/workspace/desk-1/report.md",
      tool_call: toolCall,
    }],
    provider_incidents: {},
  };
}

function specSnapshot(): Snapshot {
  const value = snapshot();
  const uri = "/Users/alice/.agent/workspace/desk-1/customer-segmentation_SPEC.md";
  return {
    ...value,
    timeline: [{ item: "artifact", id: `doc:${uri}` }],
    artifacts: [{
      ...value.artifacts[0]!,
      id: `doc:${uri}`,
      title: "customer-segmentation_SPEC.md",
      uri,
    }],
  };
}

beforeEach(() => {
  installProductModule({
    ...neutralProduct,
    artifacts: {
      ...neutralProduct.artifacts,
      isCloudUri: (uri) => /^\/product-artifacts\/[^/]+$/.test(uri),
    },
  });
  resetArtifactCloudSync();
  configureArtifactCloudCredentials(creds);
  invoke.mockReset();
});
afterEach(() => installProductModule(neutralProduct));

describe("mandatory generated artifact cloud sync", () => {
  it("persists a safe retry URI and never serializes the absolute host path", () => {
    const cloud = snapshotForArtifactCloud("desk-1", snapshot());
    expect(cloud.artifacts[0]).toMatchObject({
      id: "doc:report.md",
      uri: "workspace-artifact://desk-1/report.md",
    });
    expect(cloud.timeline[0]).toEqual({ item: "artifact", id: "doc:report.md" });
    expect(JSON.stringify(cloud)).not.toContain("/Users/alice");
  });

  it("replaces the retry URI only after native upload completion", async () => {
    mockUpload(() => ({
      artifact_id: "deskart_1",
      logical_id: "doc:report.md",
      filename: "report.md",
      content_type: "text/markdown",
      size_bytes: 12,
      sha256: "a".repeat(64),
      state: "uploaded",
      uri: "/product-artifacts/deskart_1",
    }));
    const onReady = vi.fn();
    const local = snapshot();
    scheduleArtifactCloudSync(creds, "desk-1", local, onReady);
    await vi.waitFor(() => expect(onReady).toHaveBeenCalledOnce());

    expect(invoke).toHaveBeenCalledWith("product_request", {
      operation: "artifact.upload",
      payload: {
        desktopId: "desk-1",
        logicalId: "doc:report.md",
        sourceUri: "workspace-artifact://desk-1/.clark-sync/staged-report.md",
      },
    });
    expect(snapshotForArtifactCloud("desk-1", local).artifacts[0].uri)
      .toBe("/product-artifacts/deskart_1");
  });

  it("declares restart safety after native staging without waiting for cloud upload", async () => {
    mockUpload(() => new Promise(() => {}));
    scheduleArtifactCloudSync(creds, "desk-1", snapshot(), () => {});

    await expect(prepareArtifactCloudDurability(100)).resolves.toBe(true);
    expect(productCalls()).toHaveLength(1);
  });

  it("waits for an idle snapshot before finalizing the temporary Spec file", async () => {
    mockUpload(() => ({
      artifact_id: "spec:1",
      logical_id: "doc:customer-segmentation_SPEC.md",
      filename: "customer-segmentation_SPEC.md",
      content_type: "text/markdown",
      size_bytes: 12,
      sha256: "a".repeat(64),
      state: "uploaded",
      uri: "/product-artifacts/spec-1",
    }));
    const local = specSnapshot();
    scheduleArtifactCloudSync(creds, "desk-1", local, () => {}, undefined, false);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(invoke).not.toHaveBeenCalled();

    scheduleArtifactCloudSync(creds, "desk-1", local, () => {}, undefined, true);
    await vi.waitFor(() => expect(productCalls()).toHaveLength(1));
  });

  it("treats a rewrite from a later tool call as a new immutable version", async () => {
    let uploads = 0;
    mockUpload((args) => ({
      artifact_id: `deskart_${++uploads}`,
      logical_id: args.payload.logicalId,
      filename: "report.md",
      content_type: "text/markdown",
      size_bytes: 12,
      sha256: "b".repeat(64),
      state: "uploaded",
      uri: `/product-artifacts/deskart_${uploads}`,
    }));
    scheduleArtifactCloudSync(creds, "desk-1", snapshot("tool-1"), () => {});
    await vi.waitFor(() => expect(productCalls()).toHaveLength(1));
    scheduleArtifactCloudSync(creds, "desk-1", snapshot("tool-2"), () => {});
    await vi.waitFor(() => expect(productCalls()).toHaveLength(2));
  });

  it("revalidates the same path at the idle byte boundary", async () => {
    let stage = 0;
    let uploads = 0;
    invoke.mockImplementation(async (command, args) => {
      if (command === "desktop_artifact_stage") {
        stage += 1;
        return {
          sourceUri: `workspace-artifact://desk-1/.clark-sync/stage-${stage}.md`,
          sha256: String(stage).repeat(64),
          remoteUri: null,
        };
      }
      if (command === "desktop_artifact_mark_uploaded") return undefined;
      if (command === "product_request") {
        uploads += 1;
        return {
          artifact_id: `deskart_${uploads}`,
          logical_id: args.payload.logicalId,
          filename: "report.md",
          content_type: "text/markdown",
          size_bytes: 12,
          sha256: String(uploads).repeat(64),
          state: "uploaded",
          uri: `/product-artifacts/deskart_${uploads}`,
        };
      }
      throw new Error(`unexpected command ${command}`);
    });
    const local = snapshot();

    scheduleArtifactCloudSync(creds, "desk-1", local, () => {}, undefined, true);
    await vi.waitFor(() => expect(productCalls()).toHaveLength(1));
    expect(snapshotForArtifactCloud("desk-1", local).artifacts[0].uri)
      .toBe("/product-artifacts/deskart_1");

    scheduleArtifactCloudSync(creds, "desk-1", local, () => {}, undefined, true);
    await vi.waitFor(() => expect(productCalls()).toHaveLength(2));
    expect(invoke).toHaveBeenCalledWith("desktop_artifact_mark_uploaded", {
      desktopId: "desk-1",
      logicalId: "doc:report.md",
      sha256: "2".repeat(64),
      remoteUri: "/product-artifacts/deskart_2",
    });
    expect(snapshotForArtifactCloud("desk-1", local).artifacts[0].uri)
      .toBe("/product-artifacts/deskart_2");
  });

  it("does not let an older upload acknowledge replacement bytes", async () => {
    let resolveFirst!: (artifact: unknown) => void;
    let uploads = 0;
    invoke.mockImplementation(async (command, args) => {
      if (command === "desktop_artifact_stage") {
        const stage = invoke.mock.calls.filter(([name]) => name === "desktop_artifact_stage").length;
        return {
          sourceUri: `workspace-artifact://desk-1/.clark-sync/stage-${stage}.md`,
          sha256: String(stage).repeat(64),
          remoteUri: null,
        };
      }
      if (command === "desktop_artifact_mark_uploaded") return undefined;
      if (command === "product_request") {
        uploads += 1;
        if (uploads === 1) return new Promise((resolve) => { resolveFirst = resolve; });
        return {
          artifact_id: "deskart_new",
          logical_id: args.payload.logicalId,
          filename: "report.md",
          content_type: "text/markdown",
          size_bytes: 12,
          sha256: "2".repeat(64),
          state: "uploaded",
          uri: "/product-artifacts/deskart_new",
        };
      }
      throw new Error(`unexpected command ${command}`);
    });
    const onReady = vi.fn();
    const local = snapshot();
    scheduleArtifactCloudSync(creds, "desk-1", local, onReady, undefined, true);
    await vi.waitFor(() => expect(productCalls()).toHaveLength(1));

    scheduleArtifactCloudSync(creds, "desk-1", local, onReady, undefined, true);
    resolveFirst({
      artifact_id: "deskart_old",
      logical_id: "doc:report.md",
      filename: "report.md",
      content_type: "text/markdown",
      size_bytes: 12,
      sha256: "1".repeat(64),
      state: "uploaded",
      uri: "/product-artifacts/deskart_old",
    });

    await vi.waitFor(() => expect(productCalls()).toHaveLength(2));
    await vi.waitFor(() => expect(onReady).toHaveBeenCalledOnce());
    const marks = invoke.mock.calls.filter(([command]) => command === "desktop_artifact_mark_uploaded");
    expect(marks).toEqual([["desktop_artifact_mark_uploaded", {
      desktopId: "desk-1",
      logicalId: "doc:report.md",
      sha256: "2".repeat(64),
      remoteUri: "/product-artifacts/deskart_new",
    }]]);
    expect(snapshotForArtifactCloud("desk-1", local).artifacts[0].uri)
      .toBe("/product-artifacts/deskart_new");
  });

  it("fences an in-flight completion when the conversation is deleted", async () => {
    let resolveUpload!: (artifact: unknown) => void;
    mockUpload(() => new Promise((resolve) => {
      resolveUpload = resolve;
    }));
    const onReady = vi.fn();
    const local = snapshot();
    scheduleArtifactCloudSync(creds, "desk-1", local, onReady);
    await vi.waitFor(() => expect(productCalls()).toHaveLength(1));
    forgetArtifactCloudConversation("desk-1");
    resolveUpload({
      artifact_id: "deskart_late",
      logical_id: "doc:report.md",
      filename: "report.md",
      content_type: "text/markdown",
      size_bytes: 12,
      sha256: "c".repeat(64),
      state: "uploaded",
      uri: "/product-artifacts/deskart_late",
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(onReady).not.toHaveBeenCalled();
    expect(snapshotForArtifactCloud("desk-1", local).artifacts[0].uri)
      .toBe("workspace-artifact://desk-1/report.md");
  });

  it("surfaces a permanent quota failure without an endless retry loop", async () => {
    mockUpload(() => Promise.reject(
      new Error("artifact initiation failed (403 Forbidden): quota exceeded"),
    ));
    const warning = vi.fn();
    const local = snapshot();
    scheduleArtifactCloudSync(creds, "desk-1", local, () => {}, warning);
    await vi.waitFor(() => expect(warning).toHaveBeenCalledOnce());

    scheduleArtifactCloudSync(creds, "desk-1", local, () => {}, warning);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(productCalls()).toHaveLength(1);
  });
});
