import { useCallback, useEffect, useRef, useState } from "react";

import type {
  CoreBridge,
  RemoteExecutorTarget,
  SkillCatalogSnapshot,
} from "../core-bridge/bridge";

export function useSkillCatalog(
  bridge: CoreBridge | null,
  cwd: string,
  remote: RemoteExecutorTarget | null,
  enabled: boolean,
) {
  const [catalog, setCatalog] = useState<SkillCatalogSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const checking = useRef(false);

  const reload = useCallback(async () => {
    if (!bridge?.reloadSkills || !cwd.trim()) return null;
    setLoading(true);
    try {
      const next = await bridge.reloadSkills(cwd, remote);
      setCatalog(next);
      setError(null);
      return next;
    } catch (cause) {
      setError(String(cause));
      return null;
    } finally {
      setLoading(false);
    }
  }, [bridge, cwd, remote]);

  useEffect(() => {
    if (!enabled || !bridge?.listSkills || !cwd.trim()) {
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
    if (!enabled || !bridge?.skillChanges || !catalog || !cwd.trim()) return;
    const timer = window.setInterval(() => {
      if (checking.current || document.visibilityState !== "visible") return;
      checking.current = true;
      void bridge
        .skillChanges!(cwd, catalog.revision, remote)
        .then((change) => {
          if (change.changed && change.snapshot) setCatalog(change.snapshot);
        })
        .catch((cause) => setError(String(cause)))
        .finally(() => {
          checking.current = false;
        });
    }, 4_000);
    return () => window.clearInterval(timer);
  }, [bridge, catalog, cwd, enabled, remote]);

  return { catalog, setCatalog, error, loading, reload };
}
