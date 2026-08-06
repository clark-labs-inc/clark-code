import { beforeEach, describe, expect, it } from "vitest";
import {
  clearComposerDraftIfUnchanged,
  composerDraftOwner,
  loadComposerDraft,
  loadComposerDraftRecord,
  markComposerDraftSynced,
  moveComposerDraft,
  removeComposerDraft,
  saveComposerDraft,
  shouldUseCloudComposerDraft,
} from "./composerDraft";

const owner = "account-one";

describe("composer drafts", () => {
  beforeEach(() => localStorage.clear());

  it("keeps each conversation's draft independent and reloadable", () => {
    saveComposerDraft(owner, "chat-one", "first chat draft");
    saveComposerDraft(owner, "chat-two", "second chat draft");

    expect(loadComposerDraft(owner, "chat-one")).toBe("first chat draft");
    expect(loadComposerDraft(owner, "chat-two")).toBe("second chat draft");
  });

  it("keeps drafts isolated between accounts", () => {
    saveComposerDraft("account-one", "shared-chat-id", "first account");
    saveComposerDraft("account-two", "shared-chat-id", "second account");

    expect(loadComposerDraft("account-one", "shared-chat-id")).toBe("first account");
    expect(loadComposerDraft("account-two", "shared-chat-id")).toBe("second account");
  });

  it("removes a draft after its conversation is permanently deleted", () => {
    saveComposerDraft(owner, "chat-one", "obsolete draft");
    removeComposerDraft(owner, "chat-one");

    expect(loadComposerDraft(owner, "chat-one")).toBe("");
  });

  it("moves a new-chat draft into the session created for its first prompt", () => {
    saveComposerDraft(owner, null, "first prompt");
    moveComposerDraft(owner, null, "created-chat", "first prompt");

    expect(loadComposerDraft(owner, null)).toBe("");
    expect(loadComposerDraft(owner, "created-chat")).toBe("first prompt");
  });

  it("does not seed a created conversation draft from an accepted first prompt", () => {
    saveComposerDraft(owner, null, "/goal finish the work");

    expect(clearComposerDraftIfUnchanged(owner, null, "/goal finish the work")).toBe(true);

    expect(loadComposerDraft(owner, null)).toBe("");
    expect(loadComposerDraft(owner, "created-chat")).toBe("");
  });

  it("keeps Scout and Security pre-conversation drafts cloud-key isolated", () => {
    saveComposerDraft(owner, "specialist:scout:new", "map the production edge");
    saveComposerDraft(owner, "specialist:security:new", "deep scan auth");

    expect(loadComposerDraft(owner, "specialist:scout:new")).toBe("map the production edge");
    expect(loadComposerDraft(owner, "specialist:security:new")).toBe("deep scan auth");
    expect(loadComposerDraft(owner, null)).toBe("");
  });

  it("uses stable account identity precedence", () => {
    expect(composerDraftOwner({
      id: "account-id",
      email: "USER@example.com",
      name: "User",
      method: "google",
    })).toBe("account-id");
    expect(composerDraftOwner({
      email: "USER@example.com",
      name: "User",
      method: "google",
    })).toBe("user@example.com");
    expect(composerDraftOwner(null)).toBe("signed-out");
  });

  it("clears an accepted draft", () => {
    saveComposerDraft(owner, "chat-one", "ready to send");
    expect(clearComposerDraftIfUnchanged(owner, "chat-one", "ready to send")).toBe(true);

    expect(loadComposerDraft(owner, "chat-one")).toBe("");
  });

  it("preserves newer text typed while an earlier draft is being accepted", () => {
    saveComposerDraft(owner, "chat-one", "submitted text");
    saveComposerDraft(owner, "chat-one", "new follow-up typed in flight");

    expect(clearComposerDraftIfUnchanged(owner, "chat-one", "submitted text")).toBe(false);
    expect(loadComposerDraft(owner, "chat-one")).toBe("new follow-up typed in flight");
  });

  it("migrates legacy plain text into a revision-aware local envelope", () => {
    localStorage.setItem(
      "clark.composer-draft.v1.account-one.chat-one",
      "legacy draft",
    );
    expect(loadComposerDraftRecord(owner, "chat-one")).toEqual({
      text: "legacy draft",
      updatedAt: 0,
      cloudRev: 0,
    });

    saveComposerDraft(owner, "chat-one", "edited locally");
    expect(loadComposerDraftRecord(owner, "chat-one")).toMatchObject({
      text: "edited locally",
      cloudRev: 0,
    });
  });

  it("advances a cloud revision without clobbering a newer local edit", () => {
    saveComposerDraft(owner, "chat-one", "sent to cloud");
    expect(markComposerDraftSynced(owner, "chat-one", "sent to cloud", 3)).toBe(true);
    saveComposerDraft(owner, "chat-one", "new local edit");
    expect(markComposerDraftSynced(owner, "chat-one", "sent to cloud", 4)).toBe(false);
    expect(loadComposerDraftRecord(owner, "chat-one")).toMatchObject({
      text: "new local edit",
      cloudRev: 3,
    });
  });

  it("hydrates only when the cloud version is actually newer", () => {
    const local = { text: "local", updatedAt: 200, cloudRev: 2 };
    expect(shouldUseCloudComposerDraft(local, { text: "cloud", updatedAt: 201 })).toBe(true);
    expect(shouldUseCloudComposerDraft(local, { text: "cloud", updatedAt: 199 })).toBe(false);
    expect(shouldUseCloudComposerDraft(local, { text: "local", updatedAt: 300 })).toBe(false);
  });

  it("keeps a recent updatedAt after clearing so a stale cloud draft cannot resurrect a just-sent message", () => {
    saveComposerDraft(owner, "chat-one", "message I just sent");
    const beforeClear = Date.now();
    expect(clearComposerDraftIfUnchanged(owner, "chat-one", "message I just sent")).toBe(true);

    const cleared = loadComposerDraftRecord(owner, "chat-one");
    expect(cleared.text).toBe("");
    // The clear must be recorded as a recent local edit, not a missing key
    // (`updatedAt: 0`), otherwise a stale cloud draft wins on timestamp.
    expect(cleared.updatedAt).toBeGreaterThanOrEqual(beforeClear);

    // The cloud still holds the just-sent text because the discard PUT hasn't
    // landed; its timestamp predates the local clear, so it must lose.
    expect(shouldUseCloudComposerDraft(cleared, {
      text: "message I just sent",
      updatedAt: beforeClear - 1000,
    })).toBe(false);
    // A genuinely newer cross-device edit still hydrates.
    expect(shouldUseCloudComposerDraft(cleared, {
      text: "edit from another device",
      updatedAt: cleared.updatedAt + 1000,
    })).toBe(true);
  });
});
