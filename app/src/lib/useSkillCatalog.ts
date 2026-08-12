import { useCallback, useEffect, useRef, useState } from "react";

import type {
  CoreBridge,
  RemoteWorkerTarget,
  SkillCatalogSnapshot,
} from "../core-bridge/bridge";

export function useSkillCatalog(
  bridge: CoreBridge | null,
  cwd: string,
  remote: RemoteWorkerTarget | null,
  enabled: boolean,
) {
  const [catalog, setCatalog] = useState<SkillCatalogSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const checking = useRef(false);
  const contextKey = `${enabled}\u0000${cwd.trim()}\u0000${remote?.id ?? "local"}`;
  const contextRef = useRef(contextKey);
  contextRef.current = contextKey;

  const reload = useCallback(async () => {
    if (!bridge?.reloadSkills) return null;
    const requestContext = contextKey;
    setLoading(true);
    try {
      const next = await bridge.reloadSkills(cwd, remote);
      if (contextRef.current !== requestContext) return null;
      setCatalog(next);
      setError(null);
      return next;
    } catch (cause) {
      if (contextRef.current !== requestContext) return null;
      setError(String(cause));
      return null;
    } finally {
      if (contextRef.current === requestContext) setLoading(false);
    }
  }, [bridge, contextKey, cwd, remote]);

  useEffect(() => {
    if (!enabled || !bridge?.listSkills) {
      setCatalog(null);
      return;
    }
    let alive = true;
    setLoading(true);
    void bridge
      .listSkills(cwd, remote)
      .then((next) => {
        if (!alive) return;
        setCatalog(next);
        setError(null);
      })
      .catch((cause) => {
        if (alive) setError(String(cause));
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [bridge, cwd, remote, enabled]);

  useEffect(() => {
    if (!enabled || !bridge?.onSkillsChanged) return;
    return bridge.onSkillsChanged((next) => {
      if (next.projectRoot === cwd) setCatalog(next);
    });
  }, [bridge, cwd, enabled]);

  useEffect(() => {
    if (!enabled || !bridge?.skillChanges || !catalog) return;
    const requestContext = contextKey;
    const timer = window.setInterval(() => {
      if (checking.current || document.visibilityState !== "visible") return;
      checking.current = true;
      void bridge
        .skillChanges!(cwd, catalog.revision, remote)
        .then((change) => {
          if (
            contextRef.current === requestContext
            && change.changed
            && change.snapshot
          ) setCatalog(change.snapshot);
        })
        .catch((cause) => {
          if (contextRef.current === requestContext) setError(String(cause));
        })
        .finally(() => {
          checking.current = false;
        });
    }, 4_000);
    return () => window.clearInterval(timer);
  }, [bridge, catalog, contextKey, cwd, enabled, remote]);

  return { catalog, setCatalog, error, loading, reload };
}
