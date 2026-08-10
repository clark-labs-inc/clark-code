import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import {
  registerSecurityRepository,
  syncSecurityInsights,
  syncSecurityScans,
  type SecurityRepositoryRegistration,
} from "./securityCloud";

const creds = {
  accountScope: "id:account-one",
};

const registration: SecurityRepositoryRegistration = {
  repository: {
    id: "repo-1",
    organizationId: "org-1",
    fingerprint: `git:${"a".repeat(64)}`,
    githubManaged: false,
  },
  repositoryPolicy: {
    policyId: "policy-1",
    status: "active",
    scheduleIntervalMinutes: 1_440,
  },
};

describe("Security scanner cloud boundary", () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it("registers with the product account bearer before local evidence sync", async () => {
    invoke.mockResolvedValue(registration);
    await registerSecurityRepository(creds, "org-1", "/work/service");
    expect(invoke).toHaveBeenCalledWith(
      "product_request",
      {
        operation: "security.register_repository",
        payload: {
          organizationId: "org-1",
          cwd: "/work/service",
        },
      },
    );
  });

  it("keeps Clark Code credential entirely behind the native command", async () => {
    invoke.mockResolvedValue({
      sealedScanCount: 1,
      syncedCount: 1,
      alreadySyncedCount: 0,
      pendingCount: 0,
      failedCount: 0,
      scans: [],
    });
    await syncSecurityScans(
      creds,
      "org-1",
      registration,
      "/work/service",
    );
    expect(invoke).toHaveBeenCalledWith("product_request", {
      operation: "security.sync_scans",
      payload: {
        organizationId: "org-1",
        repositoryId: "repo-1",
        policyId: "policy-1",
        cwd: "/work/service",
      },
    });
  });

  it("ingests sealed local scans before Security insights are queried", async () => {
    const fingerprint = `git:${"b".repeat(64)}`;
    invoke
      .mockResolvedValueOnce({
        root: "/work/service",
        fingerprint,
        canonicalRemote: "https://github.com/acme/service",
      })
      .mockResolvedValueOnce(registration)
      .mockResolvedValueOnce({
        sealedScanCount: 1,
        syncedCount: 1,
        alreadySyncedCount: 0,
        pendingCount: 0,
        failedCount: 0,
        scans: [],
      });

    const result = await syncSecurityInsights(
      creds,
      "org-1",
      "/work/service",
    );

    expect(result?.syncedCount).toBe(1);
    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
      "repository_inspect",
      "product_request",
      "product_request",
    ]);
  });
});
