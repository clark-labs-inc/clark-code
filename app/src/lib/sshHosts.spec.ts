import { describe, expect, it } from "vitest";
import { newlyAddedSshHostId, type SshHost } from "./sshHosts";

function host(id: string): SshHost {
  return { id, label: id, host: id, remoteRoot: `/workspace/${id}` };
}

describe("newlyAddedSshHostId", () => {
  it("selects the latest host introduced by the edit", () => {
    expect(
      newlyAddedSshHostId([host("existing"), host("new-1"), host("new-2")], [host("existing")]),
    ).toBe("new-2");
  });

  it("does not change selection when only existing hosts were edited", () => {
    expect(newlyAddedSshHostId([host("existing")], [host("existing")])).toBeNull();
  });
});
