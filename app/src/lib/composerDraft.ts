const DRAFT_KEY_PREFIX = "agent-desktop.composer-draft.v1.";
const DRAFT_ENVELOPE_VERSION = 3;
const NEW_CONVERSATION_DRAFT_KEY = "new.v3";

/** Versioned because older builds copied one unbound draft across product
 * surfaces, v2 could leave an accepted native-input prefix behind during the
 * first-session remount, and v3 carries a revision anchor (see `DraftAck`)
 * instead of reconciling on wall-clock time. Real conversation IDs stay
 * stable; only fresh composers move to the clean namespace. */
export function specialistStartComposerDraftId(kind: string): string {
  return `specialist:${kind}:${NEW_CONVERSATION_DRAFT_KEY}`;
}

/** The server state this device has confirmed via an acknowledged write. It is
 * the only authority a client can trust about "what does the cloud hold", so
 * reconciliation compares revisions and acknowledged text — never clocks. */
export interface DraftAck {
  /** Server revision that held `text`. */
  rev: number;
  /** Text the server held at `rev` per our last acknowledgement. */
  text: string;
}

/** A locally persisted composer draft. `text` is what the textarea shows.
 * `lastAcked` is null until this device has successfully written (or adopted)
 * a cloud revision, which means empty drafts and never-synced drafts have no
 * anchor and are treated as purely local. */
export interface ComposerDraftRecord {
  text: string;
  lastAcked: DraftAck | null;
}

/** A resolved comparison between the local draft and the current cloud row.
 * Pure: the caller persists any adoption/acknowledgement that this chooses. */
export type ComposerDraftReconcile =
  | { outcome: "local" }
  | { outcome: "acknowledge"; text: string; rev: number }
  | { outcome: "adopt"; text: string; rev: number }
  | { outcome: "conflict"; text: string; rev: number };

/** Decide who owns the composer text without trusting any clock.

 * Rules, in order:
 * - No cloud row → keep local (nothing to conflict with).
 * - Cloud revision is not newer than our last acknowledgement → keep local.
 * - Texts already agree → keep local text but record the newer revision.
 * - Local has no unacked edit → adopt the newer cloud text.
 * - Otherwise → a genuine two-sided edit; keep local and surface a conflict.
 */
export function reconcileComposerDraft(
  local: ComposerDraftRecord,
  remote: { text: string; rev: number } | null,
): ComposerDraftReconcile {
  if (!remote) return { outcome: "local" };

  const lastRev = local.lastAcked?.rev ?? 0;
  const remoteIsNewer = remote.rev > lastRev;

  if (!remoteIsNewer) return { outcome: "local" };
  if (remote.text === local.text) {
    return { outcome: "acknowledge", text: remote.text, rev: remote.rev };
  }

  const hasUnackedLocalEdit = local.text !== (local.lastAcked?.text ?? "");
  if (!hasUnackedLocalEdit) {
    return { outcome: "adopt", text: remote.text, rev: remote.rev };
  }
  return { outcome: "conflict", text: remote.text, rev: remote.rev };
}

interface DraftOwner {
  id?: string;
  email?: string;
  name: string;
  method: string;
}

/** Stable local namespace so drafts never bleed between accounts on one device. */
export function composerDraftOwner(user: DraftOwner | null): string {
  if (!user) return "signed-out";
  return user.id?.trim()
    || user.email?.trim().toLowerCase()
    || `${user.method}:${user.name.trim()}`;
}

function draftKey(owner: string, conversationId: string | null): string {
  return `${DRAFT_KEY_PREFIX}${encodeURIComponent(owner)}.${encodeURIComponent(conversationId ?? NEW_CONVERSATION_DRAFT_KEY)}`;
}

/** Read the local draft envelope, migrating legacy shapes up to v3.

 * v1 stored bare text; v2 stored `{ text, updatedAt, cloudRev }`. v3 stores
 * `{ text, lastAcked }`. Legacy records keep their last acknowledged revision
 * (when one existed) as `{ rev: cloudRev, text }`; the stale clock field is
 * dropped because reconciliation never needs it. */
export function loadComposerDraftRecord(
  owner: string,
  conversationId: string | null,
): ComposerDraftRecord {
  try {
    const stored = localStorage.getItem(draftKey(owner, conversationId));
    if (!stored) return { text: "", lastAcked: null };
    try {
      const parsed = JSON.parse(stored) as Partial<ComposerDraftRecord> & {
        version?: number;
        cloudRev?: number;
      };
      if (
        parsed.version === DRAFT_ENVELOPE_VERSION
        && typeof parsed.text === "string"
        && (typeof parsed.lastAcked === "undefined" || isDraftAck(parsed.lastAcked))
      ) {
        return { text: parsed.text, lastAcked: parsed.lastAcked ?? null };
      }
      if (
        typeof parsed.text === "string"
        && typeof parsed.cloudRev === "number"
      ) {
        return {
          text: parsed.text,
          lastAcked: parsed.cloudRev > 0
            ? { rev: parsed.cloudRev, text: parsed.text }
            : null,
        };
      }
    } catch {
      // Pre-envelope drafts were stored as unescaped plain text.
    }
    return { text: stored, lastAcked: null };
  } catch {
    return { text: "", lastAcked: null };
  }
}

/** Read the locally persisted text for one conversation's composer. */
export function loadComposerDraft(owner: string, conversationId: string | null): string {
  return loadComposerDraftRecord(owner, conversationId).text;
}

function isDraftAck(value: unknown): value is DraftAck | null {
  return value === null
    || (typeof value === "object" && value !== null
      && typeof (value as DraftAck).rev === "number"
      && typeof (value as DraftAck).text === "string");
}

function writeComposerDraftRecord(
  owner: string,
  conversationId: string | null,
  record: ComposerDraftRecord,
): void {
  const key = draftKey(owner, conversationId);
  localStorage.setItem(key, JSON.stringify({
    version: DRAFT_ENVELOPE_VERSION,
    ...record,
  }));
}

/** Persist a draft separately for each conversation. Typing does not advance
 * the last acknowledgement: the cloud still holds whatever was last written
 * there until a save succeeds, so `lastAcked` is preserved verbatim. */
export function saveComposerDraft(
  owner: string,
  conversationId: string | null,
  text: string,
): void {
  try {
    const current = loadComposerDraftRecord(owner, conversationId);
    writeComposerDraftRecord(owner, conversationId, { text, lastAcked: current.lastAcked });
  } catch {
    // Draft persistence is best-effort: a full or unavailable local store must
    // never prevent the composer from accepting input.
  }
}

/** Record that the server now holds `text` at `rev` because this device wrote
 * or adopted it. Returns false when a newer local edit landed first so the
 * anchor is not falsely attributed to text the server has not seen. */
export function acknowledgeComposerDraft(
  owner: string,
  conversationId: string | null,
  ack: DraftAck,
): boolean {
  try {
    const current = loadComposerDraftRecord(owner, conversationId);
    if (current.text !== ack.text) return false;
    writeComposerDraftRecord(owner, conversationId, {
      text: current.text,
      lastAcked: ack,
    });
    return true;
  } catch {
    return false;
  }
}

/** Adopt a cloud value as both the visible text and the new acknowledgement. */
export function adoptComposerDraft(
  owner: string,
  conversationId: string | null,
  text: string,
  rev: number,
): void {
  try {
    writeComposerDraftRecord(owner, conversationId, {
      text,
      lastAcked: { rev, text },
    });
  } catch {
    // Cloud hydration is best-effort for the same reason as local persistence.
  }
}

/** Accept a cloud value as the acknowledgement without disturbing local text. */
export function adoptComposerDraftAck(
  owner: string,
  conversationId: string | null,
  ack: DraftAck,
): void {
  try {
    const current = loadComposerDraftRecord(owner, conversationId);
    writeComposerDraftRecord(owner, conversationId, {
      text: current.text,
      lastAcked: ack,
    });
  } catch {
    // Cloud hydration is best-effort for the same reason as local persistence.
  }
}

export function removeComposerDraft(owner: string, conversationId: string): void {
  saveComposerDraft(owner, conversationId, "");
}

export function moveComposerDraft(
  owner: string,
  fromConversationId: string | null,
  toConversationId: string,
  text: string,
): void {
  // The destination is a different cloud key, so its cloud row is unknown and
  // must not inherit the source's acknowledgement; reconciliation will re-read
  // the destination row on the next mount.
  try {
    writeComposerDraftRecord(owner, toConversationId, { text, lastAcked: null });
  } catch {
    saveComposerDraft(owner, toConversationId, text);
  }
  if (fromConversationId !== toConversationId) {
    saveComposerDraft(owner, fromConversationId, "");
  }
}

/** Clear only the exact draft that was accepted, preserving edits made in flight. */
export function clearComposerDraftIfUnchanged(
  owner: string,
  conversationId: string | null,
  submittedText: string,
): boolean {
  if (loadComposerDraft(owner, conversationId) !== submittedText) return false;
  saveComposerDraft(owner, conversationId, "");
  return true;
}

/** Mirror of the composer textarea's current draft. The Composer writes this on
 *  every edit so store actions can read the unsent text without making the draft
 *  itself reactive — typing must not re-render the whole store. Persistence is
 *  owned by the exact conversation or specialist-start key; this ref must never
 *  bridge text between those scopes. Non-reactive by design — the textarea
 *  remains the source of truth. */
export const composerDraftRef: { current: string } = { current: "" };
