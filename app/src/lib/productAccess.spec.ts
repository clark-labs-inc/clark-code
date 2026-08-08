import { describe, expect, it } from "vitest";
import { capabilityAccess, parseProductAccess } from "./productAccess";

const access = {
  schema_version: 1,
  account: { kind: "workspace", label: "Example" },
  capabilities: [{
    id: "research",
    availability: "available",
    reason: "ready",
    title: "Research is ready",
    detail: "Research workflows are available.",
  }],
  usage: {
    state: "ready",
    label: "18%",
    percent_used: 18,
    is_unlimited: false,
  },
};

describe("product access contract", () => {
  it("parses an opaque provider-owned capability projection", () => {
    const parsed = parseProductAccess(access);
    expect(parsed.account).toEqual({ kind: "workspace", label: "Example" });
    expect(capabilityAccess(parsed, "research")?.availability).toBe("available");
    expect(parsed.usage.percentUsed).toBe(18);
  });

  it("rejects duplicate capability ids", () => {
    expect(() => parseProductAccess({
      ...access,
      capabilities: [access.capabilities[0], access.capabilities[0]],
    })).toThrow("must be unique");
  });

  it("rejects unversioned and out-of-range responses", () => {
    expect(() => parseProductAccess({ ...access, schema_version: 2 })).toThrow("unsupported");
    expect(() => parseProductAccess({
      ...access,
      usage: { ...access.usage, percent_used: 101 },
    })).toThrow("usage is invalid");
  });

  it("rejects unsafe product action URLs", () => {
    expect(() => parseProductAccess({
      ...access,
      capabilities: [{ ...access.capabilities[0], action_url: "javascript:alert(1)" }],
    })).toThrow("action URL is invalid");
  });
});
