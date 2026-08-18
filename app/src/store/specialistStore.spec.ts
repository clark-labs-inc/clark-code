import { afterEach, describe, expect, it } from "vitest";

import { useSpecialistStore } from "./specialistStore";

afterEach(() => {
  useSpecialistStore.setState({ active: null, expanded: null, scoutScopeOpen: false });
});

describe("specialist navigation state", () => {
  it("keeps the saved-session branch expanded when leaving the specialist workspace", () => {
    // The neutral foundation test product has no branded specialist catalog,
    // so seed the same state that the product's guarded `open` action creates.
    useSpecialistStore.setState({ active: "security", expanded: "security" });

    useSpecialistStore.getState().close();

    expect(useSpecialistStore.getState()).toMatchObject({
      active: null,
      expanded: "security",
    });
  });

  it("opens the Scout scope chooser from composer state and closes it with the lens", () => {
    useSpecialistStore.setState({ active: "scout", expanded: "scout" });

    useSpecialistStore.getState().setScoutScopeOpen(true);
    expect(useSpecialistStore.getState().scoutScopeOpen).toBe(true);

    useSpecialistStore.getState().close();
    expect(useSpecialistStore.getState().scoutScopeOpen).toBe(false);
  });

});
