import { useEffect } from "react";
import { useSessionStore } from "../store/sessionStore";

export const BILLING_REFRESH_MS = 60_000;

/** Keep plan/coverage truth fresh after checkout, renewal, cancellation,
 * workspace-seat changes, and a long-running foreground session. */
export function BillingStateSync() {
  const loadBilling = useSessionStore((state) => state.loadBilling);

  useEffect(() => {
    const refresh = () => {
      if (document.visibilityState === "visible") void loadBilling();
    };
    const onVisibility = () => {
      if (document.visibilityState === "visible") refresh();
    };
    const timer = window.setInterval(refresh, BILLING_REFRESH_MS);
    window.addEventListener("focus", refresh);
    window.addEventListener("online", refresh);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("focus", refresh);
      window.removeEventListener("online", refresh);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [loadBilling]);

  return null;
}
