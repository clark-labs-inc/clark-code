import { describe, expect, it } from "vitest";
import type { SecurityScanRecord } from "../core-bridge/types";
import type { SecurityOrganization } from "../lib/securityCloud";
import {
  previouslySelectedSecurityOrganization,
  summarizeSecurityScan,
} from "./SecurityPanel";

describe("Security scan summaries", () => {
  it("prefers sealed counts over mutable bundle rows", () => {
    const record = {
      bundle: {
        coverage: [{ status: "reviewed" }, { status: "excluded" }],
        supportingCoverage: [{ status: "reviewed" }],
      },
      seal: {
        findings: [{ findingId: "SEC-1" }],
        reviewedFiles: 7,
        excludedFiles: 2,
        supportingFiles: 3,
      },
    } as unknown as SecurityScanRecord;
    expect(summarizeSecurityScan(record)).toEqual({
      sealed: true,
      findings: 1,
      reviewed: 7,
      excluded: 2,
      supporting: 3,
    });
  });

  it("describes an unsealed bundle without inventing findings", () => {
    const record = {
      bundle: {
        coverage: [{ status: "reviewed" }, { status: "excluded" }],
        supportingCoverage: [],
      },
      seal: null,
    } as unknown as SecurityScanRecord;
    expect(summarizeSecurityScan(record)).toEqual({
      sealed: false,
      findings: 0,
      reviewed: 2,
      excluded: 1,
      supporting: 0,
    });
  });
});

describe("Security repository connection", () => {
  const organizations: SecurityOrganization[] = [
    { id: "org-one", name: "Clark Labs", role: "owner", status: "active" },
  ];

  it("requires an explicit first connection even when only one workspace exists", () => {
    expect(previouslySelectedSecurityOrganization(organizations, null)).toBeUndefined();
  });

  it("reconnects a repository to its previously selected workspace", () => {
    expect(previouslySelectedSecurityOrganization(organizations, "org-one")).toEqual(
      organizations[0],
    );
  });
});
