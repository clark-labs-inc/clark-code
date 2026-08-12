import { useCallback, useEffect, useRef, useState } from "react";
import { useSessionStore } from "../store/sessionStore";
import {
  loadComposerDraftRecord,
  markComposerDraftSynced,
  replaceComposerDraftFromCloud,
  shouldUseCloudComposerDraft,
} from "./composerDraft";
import {
  clearSubmittedCloudComposerDraft,
  cloudComposerDraftBaseRevision,
  cloudComposerDraftGet,
  cloudComposerDraftPut,
  type CloudDraftClearResult,
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
  const inflightPromiseRef = useRef<Promise<void> | null>(null);
  const conflictPausedRef = useRef(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const onHydrateRef = useRef(onHydrate);
  onHydrateRef.current = onHydrate;

  const syncLatest = useCallback((): Promise<void> => {
    const config = configRef.current;
    if (!config || !readyRef.current || conflictPausedRef.current) {
      return Promise.resolve();
    }
    if (inflightPromiseRef.current) return inflightPromiseRef.current;
    const syncPromise = (async () => {
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
            // The current revision changed after our read. Retrying against
            // that newer revision would silently overwrite another device's
            // edit. Keep the local text in its local draft record and pause
            // cloud writes for this mounted scope.
            conflictPausedRef.current = true;
            setStatus("conflict");
            return;
          }
          conflictPausedRef.current = false;
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
        inflightPromiseRef.current = null;
      }
    })();
    inflightPromiseRef.current = syncPromise;
    return syncPromise;
  }, []);

  useEffect(() => {
    generationRef.current += 1;
    const generation = generationRef.current;
    discardedGenerationRef.current = null;
    const creds = cloudCreds(auth);
    desiredTextRef.current = text;
    readyRef.current = false;
    conflictPausedRef.current = false;
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
        revRef.current = cloudComposerDraftBaseRevision(remote);
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
    const activeGeneration = configRef.current?.generation;
    if (
      activeGeneration !== undefined
      && discardedGenerationRef.current === activeGeneration
      && text
    ) {
      // `acceptSubmitted()` already made the desired cloud value empty, while
      // this effect is still seeing the previous render's non-empty prop. Do
      // not enqueue that stale render after the clear; the textarea's empty
      // render will arrive next and release the guard.
      return;
    }
    if (!text && discardedGenerationRef.current === activeGeneration) {
      discardedGenerationRef.current = null;
    }
    desiredTextRef.current = text;
    if (!readyRef.current || !configRef.current) return;
    if (conflictPausedRef.current) {
      setStatus("conflict");
      return;
    }
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
      if (
        readyRef.current
        && !conflictPausedRef.current
        && desiredTextRef.current !== syncedTextRef.current
      ) {
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

  /** Settle pending writes, then conditionally remove only the submitted text.
   * A newer edit from another device is preserved and a repeatedly rejected
   * current revision is surfaced instead of entering a tight 409 loop. */
  const acceptSubmitted = useCallback(async (
    targetConversationId: string | null,
    submittedText: string,
  ): Promise<CloudDraftClearResult | null> => {
    desiredTextRef.current = "";
    if (timerRef.current) clearTimeout(timerRef.current);
    const config = configRef.current;
    if (config && config.conversationId === targetConversationId) {
      discardedGenerationRef.current = config.generation;
      if (readyRef.current) {
        setStatus("saving");
        await syncLatest();
      } else if (inflightPromiseRef.current) {
        await inflightPromiseRef.current;
      }
    }
    const creds = cloudCreds(useSessionStore.getState().auth);
    if (!creds) return null;
    const result = await clearSubmittedCloudComposerDraft(
      creds,
      targetConversationId,
      submittedText,
    );
    const active = configRef.current;
    if (active && active.conversationId === targetConversationId) {
      revRef.current = result.draft?.rev ?? 0;
      syncedTextRef.current = result.draft?.text ?? "";
      if (result.outcome === "preserved_newer") {
        conflictPausedRef.current = true;
        desiredTextRef.current = result.draft.text;
        const updatedAt = Date.parse(result.draft.updatedAt) || Date.now();
        replaceComposerDraftFromCloud(active.owner, targetConversationId, {
          text: result.draft.text,
          updatedAt,
          cloudRev: result.draft.rev,
        });
        onHydrateRef.current(result.draft.text);
        setStatus("conflict");
      } else {
        conflictPausedRef.current = false;
        markComposerDraftSynced(
          active.owner,
          targetConversationId,
          "",
          result.draft?.rev ?? 0,
        );
        setStatus("saved");
      }
    }
    return result;
  }, [syncLatest]);

  return { status, acceptSubmitted };
}
