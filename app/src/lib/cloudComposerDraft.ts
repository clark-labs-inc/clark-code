import { invoke } from "@tauri-apps/api/core";
import type { CloudCreds } from "./cloudHistory";

const NEW_CONVERSATION_DRAFT_KEY = "new";

export interface CloudComposerDraft {
  draftKey: string;
  text: string;
  rev: number;
  updatedAt: string;
}

export type CloudDraftWrite =
  | { conflict: false; draft: CloudComposerDraft }
  | { conflict: true; draft: CloudComposerDraft };

function cloudComposerDraftKey(conversationId: string | null): string {
  return conversationId ?? NEW_CONVERSATION_DRAFT_KEY;
}

export function cloudComposerDraftGet(
  _creds: CloudCreds,
  conversationId: string | null,
): Promise<CloudComposerDraft | null> {
  return invoke<CloudComposerDraft | null>("desktop_draft_get", {
    draftKey: cloudComposerDraftKey(conversationId),
  });
}

export async function cloudComposerDraftPut(
  _creds: CloudCreds,
  conversationId: string | null,
  text: string,
  baseRev: number,
): Promise<CloudDraftWrite> {
  const result = await invoke<CloudComposerDraft | {
    conflict: true;
    current: CloudComposerDraft;
  }>("desktop_draft_put", {
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

/** Clear only the cloud version we just observed. A concurrent edit wins. */
export async function clearCloudComposerDraft(
  creds: CloudCreds,
  conversationId: string | null,
): Promise<void> {
  const current = await cloudComposerDraftGet(creds, conversationId);
  if (!current?.text) return;
  await cloudComposerDraftPut(creds, conversationId, "", current.rev);
}
