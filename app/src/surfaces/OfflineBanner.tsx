import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { WifiOff } from "lucide-react";
import { useOnline } from "../lib/online";
import { EXPAND, EXPAND_REDUCED } from "../lib/motion";

/** A thin banner shown while the machine is offline. History stays available
 *  from the local cache; new runs need the connection back. */
export function OfflineBanner() {
  const online = useOnline();
  const reduce = useReducedMotion();
  return (
    <AnimatePresence initial={false}>
      {!online && (
        <m.div
          {...(reduce ? EXPAND_REDUCED : EXPAND)}
          className="overflow-hidden border-b border-border bg-bg-secondary"
        >
          <div className="flex items-center gap-2 px-4 py-1.5 text-xs text-ink-muted">
            <WifiOff className="size-3.5 shrink-0 text-ink-faint" />
            You’re offline — past conversations are cached; new runs need a connection.
          </div>
        </m.div>
      )}
    </AnimatePresence>
  );
}
