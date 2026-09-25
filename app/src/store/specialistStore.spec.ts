import { afterEach, describe, expect, it } from "vitest";

import { contextsAfterSpecialistOpen, useSpecialistStore } from "./specialistStore";

afterEach(() => {
  useSpecialistStore.setState({ active: null, expanded: null });
});

describe("specialist navigation state", () => {
  it("does not carry composer context into an unrelated new specialist composer", () => {
    const contexts = contextsAfterSpecialistOpen(
      {
        security: {
          kind: "security",
          objectId: "previous-object",
          workflow: "security:previous",
        },
      },
      "security",
      "security:default",
    );

    expect(contexts.security).toEqual({
      kind: "security",
      workflow: "security:default",
    });
  });

  it("drops obsolete Insights scope when opening a saved Security conversation", () => {
    const contexts = contextsAfterSpecialistOpen(
      {
        security: {
          kind: "security",
          organizationId: "previous-organization",
        },
      },
      "security",
      "security:default",
      {
        kind: "security",
        objectId: "opened-conversation-object",
      },
    );

    expect(contexts.security).toEqual({
      kind: "security",
    });
  });

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

});
