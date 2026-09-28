import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emptySnapshot, type Session } from "../../core-bridge/types";
import { contextsAfterSpecialistOpen } from "../../store/specialistStore";
import { useSessionStore } from "../../store/sessionStore";
import { useSpecialistStore } from "../../store/specialistStore";
import { newSpecialistConversation } from "./newSession";

beforeEach(() => {
  vi.stubGlobal("requestAnimationFrame", vi.fn());
  useSessionStore.getState().endSession({ force: true });
  useSpecialistStore.getState().close();
});

const originalOpen = useSpecialistStore.getState().open;
afterEach(() => {
  useSpecialistStore.setState({ open: originalOpen });
  vi.unstubAllGlobals();
});

describe("new specialist session", () => {
  it("keeps the selected lens while clearing the prior session's artifacts and workflow", () => {
    // The neutral foundation test product has no signed specialist catalog.
    // Exercise the navigation boundary with the same context reset as the catalog-backed store.
    const open = vi.fn((kind: "security" | "scientist") => useSpecialistStore.setState((state) => ({
      active: kind,
      contexts: contextsAfterSpecialistOpen(state.contexts, kind, "security:default"),
    })));
    useSpecialistStore.setState({ open, active: "security", contexts: { security: { kind: "security", workflow: "deep_scan" } } });
    useSessionStore.setState({
      session: { id: "old-security-session", provider: "local" } as Session,
      snapshot: {
        ...emptySnapshot(),
        artifacts: [{ id: "old-report", kind: "file", title: "Old report", uri: "/tmp/old.json" }],
      },
    });

    newSpecialistConversation("security");

    expect(useSessionStore.getState().session).toBeNull();
    expect(useSessionStore.getState().snapshot.artifacts).toEqual([]);
    expect(useSpecialistStore.getState().active).toBe("security");
    expect(useSpecialistStore.getState().contexts.security?.workflow).toBe("security:default");
    expect(open).toHaveBeenCalledWith("security");
  });
});
