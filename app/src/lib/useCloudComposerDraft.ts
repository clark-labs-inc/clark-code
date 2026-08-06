import { useCallback, useEffect, useRef, useState } from "react";
import { useSessionStore } from "../store/sessionStore";
import {
  loadComposerDraftRecord,
  markComposerDraftSynced,
  replaceComposerDraftFromCloud,
  shouldUseCloudComposerDraft,
} from "./composerDraft";
import {
  clearCloudComposerDraft,
  cloudComposerDraftGet,
  cloudComposerDraftPut,
} from "./cloudComposerDraft";
import { cloudCreds, type CloudCreds } from "./cloudHistory";

export type ComposerDraftCloudStatus =
  | "local"
  | "loading"
  | "saving"
  | "saved"
  | "offline"
  | "conflict";

interface SyncConfig {
  creds: CloudCreds;
  owner: string;
  conversationId: string | null;
  generation: number;
}

const SAVE_DEBOUNCE_MS = 500;

export function useCloudComposerDraft({
  owner,
  conversationId,
  text,
  onHydrate,
}: {
  owner: string;
  conversationId: string | null;
  text: string;
  onHydrate: (text: string) => void;
}) {
  const auth = useSessionStore((state) => state.auth);
  const [status, setStatus] = useState<ComposerDraftCloudStatus>("local");
  const configRef = useRef<SyncConfig | null>(null);
  const generationRef = useRef(0);
  const discardedGenerationRef = useRef<number | null>(null);
  const readyRef = useRef(false);
  const desiredTextRef = useRef(text);
  const syncedTextRef = useRef("");
  const revRef = useRef(0);
  const inflightRef = useRef(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const syncLatest = useCallback(async () => {
    const config = configRef.current;
    if (!config || inflightRef.current || !readyRef.current) return;
    inflightRef.current = true;
    try {
      while (
        configRef.current?.generation === config.generation
        && desiredTextRef.current !== syncedTextRef.current
      ) {
        const sending = desiredTextRef.current;
        setStatus("saving");
        const result = await cloudComposerDraftPut(
          config.creds,
          config.conversationId,
          sending,
          revRef.current,
        );
        if (configRef.current?.generation !== config.generation) return;
        revRef.current = result.draft.rev;
        if (result.conflict) {
          if (result.draft.text === sending) {
            syncedTextRef.current = sending;
            markComposerDraftSynced(
              config.owner,
              config.conversationId,
              sending,
              result.draft.rev,
            );
            setStatus("saved");
            continue;
          }
          syncedTextRef.current = result.draft.text;
          continue;
        }
        syncedTextRef.current = sending;
        markComposerDraftSynced(
          config.owner,
          config.conversationId,
          sending,
          result.draft.rev,
        );
        setStatus("saved");
      }
    } catch {
      if (configRef.current?.generation === config.generation) setStatus("offline");
    } finally {
      inflightRef.current = false;
    }
  }, []);

  useEffect(() => {
    generationRef.current += 1;
    const generation = generationRef.current;
    discardedGenerationRef.current = null;
    const creds = cloudCreds(auth);
    desiredTextRef.current = text;
    readyRef.current = false;
    revRef.current = 0;
    syncedTextRef.current = "";
    if (timerRef.current) clearTimeout(timerRef.current);
    if (!creds) {
      configRef.current = null;
      setStatus(text ? "local" : "local");
      return;
    }
    const config = { creds, owner, conversationId, generation };
    configRef.current = config;
    setStatus("loading");
    let active = true;
    void cloudComposerDraftGet(creds, conversationId)
      .then((remote) => {
        if (!active || configRef.current?.generation !== generation) return;
        const local = loadComposerDraftRecord(owner, conversationId);
        revRef.current = remote?.rev ?? local.cloudRev;
        syncedTextRef.current = remote?.text ?? "";
        if (discardedGenerationRef.current === generation) {
          desiredTextRef.current = "";
          readyRef.current = true;
          setStatus(syncedTextRef.current ? "saving" : "saved");
          if (syncedTextRef.current) void syncLatest();
          return;
        }
        const remoteUpdatedAt = remote ? Date.parse(remote.updatedAt) || 0 : 0;
        const cloudWins = Boolean(remote && shouldUseCloudComposerDraft(local, {
          text: remote.text,
          updatedAt: remoteUpdatedAt,
        }));
        if (cloudWins && remote) {
          replaceComposerDraftFromCloud(owner, conversationId, {
            text: remote.text,
            updatedAt: remoteUpdatedAt,
            cloudRev: remote.rev,
          });
          desiredTextRef.current = remote.text;
          onHydrate(remote.text);
        } else {
          desiredTextRef.current = local.text;
        }
        readyRef.current = true;
        setStatus(desiredTextRef.current === syncedTextRef.current ? "saved" : "saving");
        if (desiredTextRef.current !== syncedTextRef.current) void syncLatest();
      })
      .catch(() => {
        if (!active || configRef.current?.generation !== generation) return;
        readyRef.current = true;
        setStatus("offline");
      });
    return () => {
      active = false;
    };
  }, [auth, conversationId, onHydrate, owner, syncLatest]);

  useEffect(() => {
    desiredTextRef.current = text;
    if (!readyRef.current || !configRef.current) return;
    if (text === syncedTextRef.current) {
      setStatus("saved");
      return;
    }
    setStatus("saving");
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => void syncLatest(), SAVE_DEBOUNCE_MS);
    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [syncLatest, text]);

  useEffect(() => {
    const retry = () => {
      if (readyRef.current && desiredTextRef.current !== syncedTextRef.current) {
        void syncLatest();
      }
    };
    const timer = window.setInterval(retry, 15_000);
    window.addEventListener("online", retry);
    window.addEventListener("blur", retry);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("online", retry);
      window.removeEventListener("blur", retry);
    };
  }, [syncLatest]);

  /** Make the current draft empty through the same serialized writer used for
   * typing. If an older value is already in flight, its loop observes the new
   * desired value and follows it with the clear instead of resurrecting it. */
  const discard = useCallback(() => {
    desiredTextRef.current = "";
    if (timerRef.current) clearTimeout(timerRef.current);
    const config = configRef.current;
    if (!config) return Promise.resolve();
    discardedGenerationRef.current = config.generation;
    if (!readyRef.current) {
      // The initial read cannot be allowed to rehydrate the accepted text, and
      // this best-effort CAS must survive an immediate Composer unmount.
      return clearCloudComposerDraft(config.creds, config.conversationId);
    }
    setStatus("saving");
    return syncLatest();
  }, [syncLatest]);

  const clearCreds = cloudCreds(auth);
  const clear = useCallback((targetConversationId: string | null) => {
    if (!clearCreds) return Promise.resolve();
    return clearCloudComposerDraft(clearCreds, targetConversationId);
  }, [clearCreds?.accountScope]);

  return { status, clear, discard };
}
