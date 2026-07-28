import { describe, expect, it } from "vitest";
import type { LocalSandboxStatus } from "../core-bridge/bridge";
import {
  readLocalSandboxStatus,
  sandboxBlocksSubmission,
  sandboxGateRequired,
  sandboxStatusForCwd,
} from "./SandboxSetupCard";

const status = (state: LocalSandboxStatus["state"]): LocalSandboxStatus => ({
  state,
  backend: "windows_restricted_token",
  reason: state === "enforced" ? null : "setup required",
  setup_available: state === "setup_required",
});

describe("Windows sandbox command-boundary gate", () => {
  it("blocks while readiness is loading or setup is incomplete", () => {
    expect(sandboxBlocksSubmission(true, null)).toBe(true);
    expect(sandboxBlocksSubmission(true, status("setup_required"))).toBe(true);
    expect(sandboxBlocksSubmission(true, status("unavailable"))).toBe(true);
  });

  it("allows submission only after enforcement, or when the gate is not required", () => {
    expect(sandboxBlocksSubmission(true, status("enforced"))).toBe(false);
    expect(sandboxBlocksSubmission(false, null)).toBe(false);
    expect(sandboxBlocksSubmission(false, status("setup_required"))).toBe(false);
  });

  it("fails closed in the packaged app before the bridge or status is ready", () => {
    expect(sandboxGateRequired({
      localTarget: true,
      remoteTarget: false,
      fullAccess: false,
      cwd: String.raw`C:\Users\tester\project`,
      nativeHost: true,
      statusSupported: false,
    })).toBe(true);
  });

  it("turns a missing native status command into an actionable error", async () => {
    await expect(
      readLocalSandboxStatus({} as never, String.raw`C:\Users\tester\project`),
    ).rejects.toThrow("cannot inspect the local command sandbox");
  });

  it("never reuses an enforced result from another project", () => {
    const observation = {
      cwd: String.raw`C:\Users\tester\first`,
      status: status("enforced"),
    };
    expect(sandboxStatusForCwd(observation, observation.cwd)?.state).toBe("enforced");
    expect(
      sandboxStatusForCwd(observation, String.raw`C:\Users\tester\second`),
    ).toBeNull();
  });

  it("does not gate remote targets, explicit Full Access, or browser-only mocks", () => {
    const base = {
      localTarget: true,
      remoteTarget: false,
      fullAccess: false,
      cwd: "/project",
      nativeHost: false,
      statusSupported: false,
    };
    expect(sandboxGateRequired({ ...base, remoteTarget: true })).toBe(false);
    expect(sandboxGateRequired({ ...base, fullAccess: true })).toBe(false);
    expect(sandboxGateRequired(base)).toBe(false);
  });
});
