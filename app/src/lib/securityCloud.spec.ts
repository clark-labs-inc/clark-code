import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import {
  registerSecurityRepository,
  syncSecurityScans,
  type SecurityRepositoryRegistration,
} from "./securityCloud";

const creds = {
  endpoint: "wss://www.clarkchat.com/ws",
  token: "clark-jwt",
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

describe("Clark Security cloud boundary", () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it("registers with the Clark account bearer before local evidence sync", async () => {
    invoke.mockResolvedValue(registration);
    await registerSecurityRepository(creds, "org-1", "/work/service");
    expect(invoke).toHaveBeenCalledWith(
      "desktop_security_register_repository",
      {
        endpoint: creds.endpoint,
        token: creds.token,
        organizationId: "org-1",
        cwd: "/work/service",
      },
    );
  });

  it("keeps the Clark JWT and Clark Code API key in distinct native fields", async () => {
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
      "ck_live_security",
      "org-1",
      registration,
      "/work/service",
    );
    expect(invoke).toHaveBeenCalledWith("desktop_security_sync_scans", {
      endpoint: creds.endpoint,
      token: creds.token,
      apiKey: "ck_live_security",
      organizationId: "org-1",
      repositoryId: "repo-1",
      policyId: "policy-1",
      cwd: "/work/service",
    });
  });
});
