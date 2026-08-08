import { useCallback, useEffect, useRef, useState } from "react";
import {
  clearComposerDraftIfUnchanged,
  loadComposerDraft,
  saveComposerDraft,
} from "./composerDraft";
import { useCloudComposerDraft } from "./useCloudComposerDraft";

export function useComposerDraftState(owner: string, conversationId: string | null) {
  const [value, setVisibleValue] = useState(() => loadComposerDraft(owner, conversationId));
  const valueRef = useRef(value);

  const setValue = useCallback(
    (next: string | ((previous: string) => string)) => {
      const text = typeof next === "function" ? next(valueRef.current) : next;
      valueRef.current = text;
      saveComposerDraft(owner, conversationId, text);
      setVisibleValue(text);
    },
    [conversationId, owner],
  );

  const hydrate = useCallback((text: string) => {
    valueRef.current = text;
    setVisibleValue(text);
  }, []);

  useEffect(() => {
    const draft = loadComposerDraft(owner, conversationId);
    valueRef.current = draft;
    setVisibleValue(draft);
  }, [conversationId, owner]);

  const cloud = useCloudComposerDraft({
    owner,
    conversationId,
    text: value,
    onHydrate: hydrate,
  });

  const acceptSubmitted = useCallback((submittedText: string) => {
    if (!clearComposerDraftIfUnchanged(owner, conversationId, submittedText)) return false;
    void cloud.discard();
    if (valueRef.current === submittedText) {
      valueRef.current = "";
      setVisibleValue("");
    }
    return true;
  }, [cloud.discard, conversationId, owner]);

  return {
    value,
    valueRef,
    setValue,
    setVisibleValue,
    acceptSubmitted,
    cloud,
  };
}
