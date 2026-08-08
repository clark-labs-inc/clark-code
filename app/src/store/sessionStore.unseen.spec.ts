import { describe, expect, it } from "vitest";
import { clearUnseenFinished, markUnseenFinished } from "./sessionStore.runtime";

// A run finishing while the user is elsewhere in the app should leave a blue
// "finished, not yet visited" marker on that sidebar row until it is opened.
describe("unseen finished-work marker", () => {
  it("marks a background conversation whose run just settled", () => {
    expect(markUnseenFinished([], "conv-1", "conv-active", false)).toEqual(["conv-1"]);
  });

  it("never marks the conversation currently on screen", () => {
    expect(markUnseenFinished([], "conv-1", "conv-1", false)).toEqual([]);
  });

  it("never marks an archived conversation (hidden from the list anyway)", () => {
    expect(markUnseenFinished([], "conv-1", "conv-active", true)).toEqual([]);
  });

  it("keeps an existing marker through a repeat turn until it is visited", () => {
    expect(markUnseenFinished(["conv-1"], "conv-1", "conv-active", false)).toEqual(["conv-1"]);
  });

  it("clears only the opened conversation, leaving other markers intact", () => {
    expect(clearUnseenFinished(["conv-1", "conv-2"], "conv-1")).toEqual(["conv-2"]);
  });

  it("opening a conversation with no marker is a no-op", () => {
    expect(clearUnseenFinished(["conv-2"], "conv-1")).toEqual(["conv-2"]);
  });
});
