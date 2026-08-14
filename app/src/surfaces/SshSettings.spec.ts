import { describe, expect, it } from "vitest";
import { sshConfigHostDetail } from "./SshSettings";

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
