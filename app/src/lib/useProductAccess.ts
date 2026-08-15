import { useCallback, useEffect, useState } from "react";
import { productAccessSnapshot, type ProductAccessProjection } from "./productAccess";

export function useProductAccess(enabled: boolean) {
  const [access, setAccess] = useState<ProductAccessProjection | null>(null);
  const [loading, setLoading] = useState(false);
  const [attempted, setAttempted] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    setAttempted(true);
    setError(null);
    try {
      setAccess(await productAccessSnapshot());
    } catch (reason) {
      setError(String(reason));
      throw reason;
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (enabled && !attempted && !loading) void reload().catch(() => undefined);
  }, [attempted, enabled, loading, reload]);

  return { access, loading, error, reload };
}
