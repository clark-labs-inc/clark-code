import { invoke } from "@tauri-apps/api/core";

import type { CloudCreds } from "./cloudHistory";

/** Mutations independent of the snapshot write pipeline. */
export async function cloudSetArchived(_c: CloudCreds, id: string, archived: boolean): Promise<void> {
  await invoke("desktop_conv_set_archived", {
    id,
    archived,
  });
}

/** Create (or fetch) the public share link for a synced conversation. */
export async function cloudShare(_c: CloudCreds, id: string): Promise<string> {
  const out = await invoke<{ share_url?: string }>("desktop_conv_share", {
    id,
  });
  if (!out.share_url) throw new Error("Clark did not return a share link.");
  return out.share_url;
}

/** Stop sharing a conversation (revokes the public link). */
export async function cloudUnshare(_c: CloudCreds, id: string): Promise<void> {
  await invoke("desktop_conv_unshare", { id });
}
