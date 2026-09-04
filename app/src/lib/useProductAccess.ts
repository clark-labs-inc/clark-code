import { useCallback, useEffect, useRef, useState } from "react";
import { productAccessSnapshot, type ProductAccessProjection } from "./productAccess";

export function useProductAccess(enabled: boolean, ownerKey: string | null = null) {
  const [access, setAccess] = useState<ProductAccessProjection | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [resultOwnerKey, setResultOwnerKey] = useState(ownerKey);
  const requestGeneration = useRef(0);

  const reload = useCallback(async () => {
    const generation = ++requestGeneration.current;
    setResultOwnerKey(ownerKey);
    setAccess(null);
    setLoading(true);
    setError(null);
    try {
      const next = await productAccessSnapshot();
      if (requestGeneration.current === generation) setAccess(next);
    } catch (reason) {
      if (requestGeneration.current === generation) setError(String(reason));
      throw reason;
    } finally {
      if (requestGeneration.current === generation) setLoading(false);
    }
  }, [ownerKey]);

  useEffect(() => {
    if (!enabled) {
      requestGeneration.current += 1;
      setAccess(null);
      setLoading(false);
      setError(null);
      return;
    }

    void reload().catch(() => undefined);
    return () => {
      requestGeneration.current += 1;
    };
  }, [enabled, ownerKey, reload]);

  const ownsResult = resultOwnerKey === ownerKey;
  return {
    access: ownsResult ? access : null,
    loading: enabled && (!ownsResult || loading),
    error: ownsResult ? error : null,
    reload,
  };
}
