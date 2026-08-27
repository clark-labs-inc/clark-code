import { beforeEach, describe, expect, it } from "vitest";
import {
  acknowledgeComposerDraft,
  adoptComposerDraft,
  adoptComposerDraftAck,
  clearComposerDraftIfUnchanged,
  composerDraftOwner,
  loadComposerDraft,
  loadComposerDraftRecord,
  moveComposerDraft,
  reconcileComposerDraft,
  removeComposerDraft,
  saveComposerDraft,
  specialistStartComposerDraftId,
} from "./composerDraft";

const owner = "account-one";

function ack(rev: number, text: string) {
  return { rev, text };
}

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

  it("clears an accepted specialist-start prompt without moving it into the session", () => {
    const start = specialistStartComposerDraftId("security");
    saveComposerDraft(owner, start, "scan the auth service now");

    expect(clearComposerDraftIfUnchanged(owner, start, "scan the auth service now")).toBe(true);
    expect(loadComposerDraft(owner, start)).toBe("");
    expect(loadComposerDraft(owner, "created-security-session")).toBe("");
  });

  it("keeps Scout and Security pre-conversation drafts cloud-key isolated", () => {
    const scout = specialistStartComposerDraftId("scout");
    const security = specialistStartComposerDraftId("security");
    saveComposerDraft(owner, scout, "map the production edge");
    saveComposerDraft(owner, security, "deep scan auth");

    expect(loadComposerDraft(owner, scout)).toBe("map the production edge");
    expect(loadComposerDraft(owner, security)).toBe("deep scan auth");
    expect(loadComposerDraft(owner, null)).toBe("");
  });

  it("leaves the contaminated legacy start-screen namespace behind", () => {
    localStorage.setItem(
      "agent-desktop.composer-draft.v1.account-one.new",
      "stale shared prompt",
    );

    expect(loadComposerDraft(owner, null)).toBe("");
    expect(specialistStartComposerDraftId("rsi")).toBe("specialist:rsi:new.v3");
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

  it("migrates legacy plain text into a revision-aware envelope", () => {
    localStorage.setItem(
      "agent-desktop.composer-draft.v1.account-one.chat-one",
      "legacy draft",
    );
    expect(loadComposerDraftRecord(owner, "chat-one")).toEqual({
      text: "legacy draft",
      lastAcked: null,
    });
  });

  it("migrates a v2 envelope, keeping its acknowledged revision but dropping its clock", () => {
    localStorage.setItem(
      "agent-desktop.composer-draft.v1.account-one.chat-one",
      JSON.stringify({
        version: 2,
        text: "synced long ago",
        updatedAt: 1_600_000_000_000,
        cloudRev: 5,
      }),
    );
    // A fully local legacy draft (never synced) has no anchor.
    expect(loadComposerDraftRecord(owner, "chat-one")).toEqual({
      text: "synced long ago",
      lastAcked: ack(5, "synced long ago"),
    });

    localStorage.setItem(
      "agent-desktop.composer-draft.v1.account-one.chat-two",
      JSON.stringify({
        version: 2,
        text: "never synced",
        updatedAt: 1_600_000_000_001,
        cloudRev: 0,
      }),
    );
    expect(loadComposerDraftRecord(owner, "chat-two")).toEqual({
      text: "never synced",
      lastAcked: null,
    });
  });

  it("keeps the last acknowledgement across typing so it stays an anchor for reconciliation", () => {
    saveComposerDraft(owner, "chat-one", "sent to cloud");
    expect(acknowledgeComposerDraft(owner, "chat-one", ack(3, "sent to cloud"))).toBe(true);

    saveComposerDraft(owner, "chat-one", "new local edit");
    expect(loadComposerDraftRecord(owner, "chat-one")).toEqual({
      text: "new local edit",
      lastAcked: ack(3, "sent to cloud"),
    });
  });

  it("advances an acknowledgement without clobbering a newer local edit", () => {
    saveComposerDraft(owner, "chat-one", "sent to cloud");
    expect(acknowledgeComposerDraft(owner, "chat-one", ack(3, "sent to cloud"))).toBe(true);

    saveComposerDraft(owner, "chat-one", "new local edit");
    expect(acknowledgeComposerDraft(owner, "chat-one", ack(4, "sent to cloud"))).toBe(false);
    expect(loadComposerDraftRecord(owner, "chat-one")).toEqual({
      text: "new local edit",
      lastAcked: ack(3, "sent to cloud"),
    });
  });

  it("adopts a cloud value as both visible text and acknowledgement", () => {
    saveComposerDraft(owner, "chat-one", "old");
    adoptComposerDraft(owner, "chat-one", "from cloud", 9);
    expect(loadComposerDraftRecord(owner, "chat-one")).toEqual({
      text: "from cloud",
      lastAcked: ack(9, "from cloud"),
    });
  });

  it("adopts only an acknowledgement without disturbing local text", () => {
    saveComposerDraft(owner, "chat-one", "local text");
    adoptComposerDraftAck(owner, "chat-one", ack(4, "local text"));
    expect(loadComposerDraftRecord(owner, "chat-one")).toEqual({
      text: "local text",
      lastAcked: ack(4, "local text"),
    });
  });

  it("resets the acknowledgement when a draft moves to a different cloud key", () => {
    saveComposerDraft(owner, null, "first prompt");
    acknowledgeComposerDraft(owner, null, ack(3, "first prompt"));
    moveComposerDraft(owner, null, "created-chat", "first prompt");

    expect(loadComposerDraftRecord(owner, "created-chat")).toEqual({
      text: "first prompt",
      lastAcked: null,
    });
  });

  describe("reconcileComposerDraft", () => {
    it("keeps local when there is no cloud row", () => {
      expect(reconcileComposerDraft(
        { text: "typed", lastAcked: null },
        null,
      )).toEqual({ outcome: "local" });
    });

    it("keeps local when the cloud revision is not newer than the last acknowledgement", () => {
      const local = { text: "edited", lastAcked: ack(5, "old") };
      expect(reconcileComposerDraft(local, { text: "other", rev: 5 })).toEqual({
        outcome: "local",
      });
      expect(reconcileComposerDraft(local, { text: "other", rev: 4 })).toEqual({
        outcome: "local",
      });
    });

    it("adopts a newer cloud text when local has no unacked edit", () => {
      const local = { text: "old", lastAcked: ack(5, "old") };
      expect(reconcileComposerDraft(local, { text: "new", rev: 6 })).toEqual({
        outcome: "adopt",
        text: "new",
        rev: 6,
      });
    });

    it("acknowledges without changing text when the two sides already agree", () => {
      const local = { text: "same", lastAcked: ack(5, "old") };
      expect(reconcileComposerDraft(local, { text: "same", rev: 6 })).toEqual({
        outcome: "acknowledge",
        text: "same",
        rev: 6,
      });
    });

    it("flags a conflict when both sides have divergent unacked edits", () => {
      const local = { text: "my follow-up", lastAcked: ack(5, "old") };
      expect(reconcileComposerDraft(local, { text: "their follow-up", rev: 6 })).toEqual({
        outcome: "conflict",
        text: "their follow-up",
        rev: 6,
      });
    });

    it("treats a never-synced local draft as an unacked edit", () => {
      const local = { text: "typed before first sync", lastAcked: null };
      expect(reconcileComposerDraft(local, { text: "from another device", rev: 1 })).toEqual({
        outcome: "conflict",
        text: "from another device",
        rev: 1,
      });
    });

    it("never trusts a clock: only revisions and acknowledged text decide the winner", () => {
      // The API carries no timestamps at all, so device-vs-server clock skew
      // (the previous bug) cannot influence the outcome. Two otherwise
      // identical inputs differ only by revision.
      const synced = { text: "old", lastAcked: ack(5, "old") };
      expect(reconcileComposerDraft(synced, { text: "new", rev: 6 })).toEqual({
        outcome: "adopt",
        text: "new",
        rev: 6,
      });

      const edited = { text: "old", lastAcked: ack(5, "old") };
      expect(reconcileComposerDraft(edited, { text: "new", rev: 5 })).toEqual({
        outcome: "local",
      });
    });
  });
});
