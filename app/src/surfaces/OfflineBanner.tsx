import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { WifiOff } from "lucide-react";
import { useOnline } from "../lib/online";
import { DUR, EASE } from "../lib/motion";

/** A thin banner shown while the machine is offline. History stays available
 *  from the local cache; new runs need the connection back. */
export function OfflineBanner() {
  const online = useOnline();
  const reduce = useReducedMotion();
  return (
    <AnimatePresence initial={false}>
      {!online && (
        <motion.div
          initial={reduce ? false : { height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          exit={reduce ? { opacity: 0, transition: { duration: 0 } } : { height: 0, opacity: 0 }}
          transition={{ duration: reduce ? 0 : DUR.fast, ease: EASE.out }}
          className="overflow-hidden border-b border-border bg-bg-secondary"
        >
          <div className="flex items-center gap-2 px-4 py-1.5 text-xs text-ink-muted">
            <WifiOff className="size-3.5 shrink-0 text-ink-faint" />
            You’re offline — past conversations are cached; new runs need a connection.
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
