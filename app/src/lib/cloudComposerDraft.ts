import { productRequest } from "../product/productBridge";
import type { CloudCreds } from "./cloudHistory";

// Start-screen drafts are an explicitly versioned lifecycle boundary. Earlier
// builds could retain a submitted native-input prefix across the first-session
// remount. Keep durable conversation IDs stable, while leaving that invalid
// start-screen state behind.
const NEW_CONVERSATION_DRAFT_KEY = "new.v3";

export interface CloudComposerDraft {
  draftKey: string;
  text: string;
  rev: number;
  updatedAt: string;
}

export type CloudDraftWrite =
  | { conflict: false; draft: CloudComposerDraft }
  | { conflict: true; draft: CloudComposerDraft };

export type CloudDraftClearResult =
  | {
    outcome: "cleared" | "already_clear";
    draft: CloudComposerDraft | null;
  }
  | {
    outcome: "preserved_newer";
    draft: CloudComposerDraft;
  };

const MAX_CLEAR_CONFLICT_ATTEMPTS = 2;

export function cloudComposerDraftKey(conversationId: string | null): string {
  return conversationId ?? NEW_CONVERSATION_DRAFT_KEY;
}

/** A successful 204 read is authoritative absence. A cached last-acknowledged
 * revision belongs to a row the service no longer has and must not prevent
 * recreation, so the base for a fresh write is the server's current revision
 * (or zero when the row is absent). */
export function cloudComposerDraftBaseRevision(remote: CloudComposerDraft | null): number {
  return remote?.rev ?? 0;
}

export function cloudComposerDraftGet(
  _creds: CloudCreds,
  conversationId: string | null,
): Promise<CloudComposerDraft | null> {
  return productRequest<CloudComposerDraft | null>("draft.get", {
    draftKey: cloudComposerDraftKey(conversationId),
  });
}

export async function cloudComposerDraftPut(
  _creds: CloudCreds,
  conversationId: string | null,
  text: string,
  baseRev: number,
): Promise<CloudDraftWrite> {
  const result = await productRequest<CloudComposerDraft | {
    conflict: true;
    current: CloudComposerDraft;
  }>("draft.put", {
    draftKey: cloudComposerDraftKey(conversationId),
    text,
    baseRev,
    mutationId: crypto.randomUUID(),
  });
  if ("current" in result) {
    return { conflict: true, draft: result.current };
  }
  return { conflict: false, draft: result as CloudComposerDraft };
}

/** True only for text written by the accepted submission itself. Long native
 * input arrives as ordered chunks, so an in-flight cloud write can contain a
 * non-empty prefix even after the complete prompt has been submitted. */
export function isSubmittedDraftResidue(currentText: string, submittedText: string): boolean {
  return currentText.length > 0
    && (currentText === submittedText || submittedText.startsWith(currentText));
}

/** Clear the latest cloud value after an explicit destructive boundary such as
 * abandoning or deleting its owning conversation. Unlike submission cleanup,
 * this intentionally advances across one concurrent revision. */
export async function clearCloudComposerDraft(
  creds: CloudCreds,
  conversationId: string | null,
): Promise<void> {
  let current = await cloudComposerDraftGet(creds, conversationId);
  if (!current?.text) return;
  for (let attempt = 0; attempt < MAX_CLEAR_CONFLICT_ATTEMPTS; attempt += 1) {
    const result = await cloudComposerDraftPut(creds, conversationId, "", current.rev);
    if (!result.conflict || !result.draft.text) return;
    current = result.draft;
  }
  throw new Error("Clark cloud draft did not accept its current revision.");
}

/** Clear an accepted prompt (or an ordered-input prefix of it) without
 * deleting an unrelated newer edit. The revision CAS still protects an edit
 * that lands between this read and write. */
export async function clearSubmittedCloudComposerDraft(
  creds: CloudCreds,
  conversationId: string | null,
  submittedText: string,
): Promise<CloudDraftClearResult> {
  let current = await cloudComposerDraftGet(creds, conversationId);
  if (!current || !current.text) {
    return { outcome: "already_clear", draft: current };
  }
  if (!isSubmittedDraftResidue(current.text, submittedText)) {
    return { outcome: "preserved_newer", draft: current };
  }

  for (let attempt = 0; attempt < MAX_CLEAR_CONFLICT_ATTEMPTS; attempt += 1) {
    const result = await cloudComposerDraftPut(
      creds,
      conversationId,
      "",
      current.rev,
    );
    if (!result.conflict) return { outcome: "cleared", draft: result.draft };
    current = result.draft;
    if (!current.text) return { outcome: "already_clear", draft: current };
    if (!isSubmittedDraftResidue(current.text, submittedText)) {
      return { outcome: "preserved_newer", draft: current };
    }
  }

  throw new Error("Clark cloud draft did not accept its current revision.");
}
