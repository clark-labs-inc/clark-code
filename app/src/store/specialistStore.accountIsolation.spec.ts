import { beforeEach, describe, expect, it } from "vitest";
import { useSpecialistStore } from "./specialistStore";

const accountOne = "id:account-one";
const accountTwo = "id:account-two";

beforeEach(() => {
  localStorage.clear();
  useSpecialistStore.getState().setAccountScope(null);
  useSpecialistStore.getState().setAccountScope(accountOne);
});

describe("specialist account isolation", () => {
  it("closes the active lens and swaps persisted context at an account boundary", () => {
    useSpecialistStore.getState().open("scout", {
      organizationId: "private-org",
      workspaceId: "private-workspace",
      targetId: "private-target",
    });

    useSpecialistStore.getState().setAccountScope(accountTwo);
    expect(useSpecialistStore.getState().active).toBeNull();
    expect(useSpecialistStore.getState().contexts).toEqual({});

    useSpecialistStore.getState().open("security", {
      organizationId: "second-org",
    });
    useSpecialistStore.getState().setAccountScope(accountOne);
    expect(useSpecialistStore.getState().active).toBeNull();
    expect(useSpecialistStore.getState().contexts.scout).toMatchObject({
      organizationId: "private-org",
      workspaceId: "private-workspace",
      targetId: "private-target",
    });
    expect(useSpecialistStore.getState().contexts.security).toBeUndefined();
  });
});
