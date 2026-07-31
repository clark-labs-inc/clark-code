import { beforeEach, describe, expect, it } from "vitest";
import { useSessionStore } from "./sessionStore";
import { composerDraftRef } from "../lib/composerDraft";

beforeEach(() => {
  useSessionStore.getState().endSession({ force: true });
  composerDraftRef.current = "";
});

describe("composer draft preservation across new sessions", () => {
  it("stages a half-typed draft as the next composer's prefill", () => {
    // The active-session composer mirrors its textarea into this ref on every
    // keystroke; endSession reads it back to bridge the remount.
    composerDraftRef.current = "fix the login bug";

    useSessionStore.getState().endSession();

    // The start-screen composer restores from this prefill on mount.
    expect(useSessionStore.getState().composerPrefill).toEqual({ text: "fix the login bug" });
  });

  it("does not stage anything when the composer was empty", () => {
    composerDraftRef.current = "   ";

    useSessionStore.getState().endSession();

    expect(useSessionStore.getState().composerPrefill).toBeNull();
  });

  it("discards the draft on sign-out so the next account starts clean", () => {
    composerDraftRef.current = "private thought";

    useSessionStore.getState().endSession({ force: true });

    expect(useSessionStore.getState().composerPrefill).toBeNull();
    expect(composerDraftRef.current).toBe("");
  });

  it("does not bleed a stale draft into a later new session after sign-out", () => {
    composerDraftRef.current = "leftover";
    useSessionStore.getState().endSession({ force: true });

    // A second new-session (no draft typed since) must not resurrect it.
    useSessionStore.getState().endSession();

    expect(useSessionStore.getState().composerPrefill).toBeNull();
  });
});
