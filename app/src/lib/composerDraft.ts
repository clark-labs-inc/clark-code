const DRAFT_KEY_PREFIX = "agent-desktop.composer-draft.v1.";
const DRAFT_ENVELOPE_VERSION = 2;

export interface ComposerDraftRecord {
  text: string;
  updatedAt: number;
  cloudRev: number;
}

export function shouldUseCloudComposerDraft(
  local: ComposerDraftRecord,
  remote: { text: string; updatedAt: number },
): boolean {
  return remote.text !== local.text && remote.updatedAt > local.updatedAt;
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
  return `${DRAFT_KEY_PREFIX}${encodeURIComponent(owner)}.${encodeURIComponent(conversationId ?? "new")}`;
}

/** Read the local draft envelope. Legacy plain-text values remain valid. */
export function loadComposerDraftRecord(
  owner: string,
  conversationId: string | null,
): ComposerDraftRecord {
  try {
    const stored = localStorage.getItem(draftKey(owner, conversationId));
    if (!stored) return { text: "", updatedAt: 0, cloudRev: 0 };
    try {
      const parsed = JSON.parse(stored) as Partial<ComposerDraftRecord> & { version?: number };
      if (
        parsed.version === DRAFT_ENVELOPE_VERSION
        && typeof parsed.text === "string"
        && typeof parsed.updatedAt === "number"
        && typeof parsed.cloudRev === "number"
      ) {
        return {
          text: parsed.text,
          updatedAt: parsed.updatedAt,
          cloudRev: parsed.cloudRev,
        };
      }
    } catch {
      // Pre-envelope drafts were stored as unescaped plain text.
    }
    return { text: stored, updatedAt: 0, cloudRev: 0 };
  } catch {
    return { text: "", updatedAt: 0, cloudRev: 0 };
  }
}

/** Read the locally persisted text for one conversation's composer. */
export function loadComposerDraft(owner: string, conversationId: string | null): string {
  return loadComposerDraftRecord(owner, conversationId).text;
}

function writeComposerDraftRecord(
  owner: string,
  conversationId: string | null,
  record: ComposerDraftRecord,
): void {
  const key = draftKey(owner, conversationId);
  // Always persist the record, even when empty. Removing the key on an empty
  // draft left `updatedAt: 0` after a send, so a stale cloud draft (the just-
  // sent text whose discard PUT hadn't landed yet) won the `shouldUseCloud-
  // ComposerDraft` timestamp comparison and re-hydrated the composer. Keeping
  // a recent `updatedAt` makes the stale remote lose, while a genuinely newer
  // cross-device edit (`updatedAt` greater than the clear) still wins.
  localStorage.setItem(key, JSON.stringify({
    version: DRAFT_ENVELOPE_VERSION,
    ...record,
  }));
}

/** Persist a draft separately for each conversation. An empty draft is written
 *  as a record with a fresh `updatedAt` (not removed) so a stale cloud draft
 *  cannot resurrect a just-sent message in the composer. */
export function saveComposerDraft(owner: string, conversationId: string | null, text: string): void {
  try {
    const current = loadComposerDraftRecord(owner, conversationId);
    writeComposerDraftRecord(owner, conversationId, {
      text,
      updatedAt: Date.now(),
      cloudRev: current.cloudRev,
    });
  } catch {
    // Draft persistence is best-effort: a full or unavailable local store must
    // never prevent the composer from accepting input.
  }
}

/** Accept a cloud version without making it look like a newer local edit. */
export function replaceComposerDraftFromCloud(
  owner: string,
  conversationId: string | null,
  record: ComposerDraftRecord,
): void {
  try {
    writeComposerDraftRecord(owner, conversationId, record);
  } catch {
    // Cloud hydration is best-effort for the same reason as local persistence.
  }
}

/** Advance the cloud revision only if the acknowledged text is still current. */
export function markComposerDraftSynced(
  owner: string,
  conversationId: string | null,
  text: string,
  cloudRev: number,
): boolean {
  try {
    const current = loadComposerDraftRecord(owner, conversationId);
    if (current.text !== text) return false;
    writeComposerDraftRecord(owner, conversationId, { ...current, cloudRev });
    return true;
  } catch {
    return false;
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
  const current = loadComposerDraftRecord(owner, fromConversationId);
  try {
    writeComposerDraftRecord(owner, toConversationId, {
      text,
      updatedAt: current.text === text ? current.updatedAt : Date.now(),
      cloudRev: 0,
    });
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
 *  itself reactive — typing must not re-render the whole store. `endSession`
 *  stages it as a `composerPrefill` to carry a half-typed message across the
 *  composer remount that starting a new session forces: the active-session
 *  composer unmounts and a fresh start-screen composer mounts, which would
 *  otherwise drop the local `useState` text. Non-reactive by design — the
 *  textarea remains the source of truth. */
export const composerDraftRef: { current: string } = { current: "" };
