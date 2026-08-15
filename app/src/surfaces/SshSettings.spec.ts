import { describe, expect, it } from "vitest";
import { hostCanSave, hostReady, type SshHost } from "../lib/sshHosts";
import { sshConfigHostDetail, sshConfigSelectionKey, sshTargetAfterSave } from "./SshSettings";

const host = (overrides: Partial<SshHost> = {}): SshHost => ({
  id: "host-1",
  label: "GPU box",
  host: "gpu",
  remoteRoot: "",
  ...overrides,
});

describe("sshConfigHostDetail", () => {
  it("shows the resolved SSH destination when config provides it", () => {
    expect(sshConfigHostDetail({
      alias: "gpu-box",
      hostname: "10.0.0.24",
      user: "ubuntu",
    })).toBe("ubuntu@10.0.0.24");
  });

  it("keeps partial and alias-only config entries readable", () => {
    expect(sshConfigHostDetail({
      alias: "production",
      hostname: "prod.internal",
      user: null,
    })).toBe("prod.internal");
    expect(sshConfigHostDetail({
      alias: "staging",
      hostname: null,
      user: "deploy",
    })).toBe("deploy@staging");
    expect(sshConfigHostDetail({
      alias: "backup",
      hostname: null,
      user: null,
    })).toBe("SSH config alias");
  });
});

describe("remote host setup", () => {
  it("allows saving a connection before choosing a default project folder", () => {
    expect(hostCanSave(host())).toBe(true);
    expect(hostReady(host())).toBe(false);
    expect(hostCanSave(host({ host: "   " }))).toBe(false);
  });

  it("does not refocus the host row while the project folder is edited", () => {
    const beforeBackspace = host({ remoteRoot: "/home/ubuntu/project" });
    const afterBackspace = host({ remoteRoot: "/home/ubuntu/projec" });

    expect(sshConfigSelectionKey("config", beforeBackspace)).toBe(
      sshConfigSelectionKey("config", afterBackspace),
    );
    expect(sshConfigSelectionKey("manual", beforeBackspace)).toBeNull();
  });

  it("returns a host added from the execution picker as the active remote target", () => {
    const previous = host({ id: "previous", host: "previous" });
    const added = host({ id: "added", host: "gpu" });

    expect(sshTargetAfterSave({
      purpose: "select_execution_target",
      selectedHostId: previous.id,
      committedHostId: added.id,
      hosts: [previous, added],
    })).toEqual({ selectedHostId: added.id, activateRemote: true });
  });

  it("keeps passive Settings management from changing the execution target", () => {
    const previous = host({ id: "previous", host: "previous" });
    const added = host({ id: "added", host: "gpu" });

    expect(sshTargetAfterSave({
      purpose: "manage",
      selectedHostId: previous.id,
      committedHostId: added.id,
      hosts: [previous, added],
    })).toEqual({ selectedHostId: previous.id, activateRemote: false });
  });
});
