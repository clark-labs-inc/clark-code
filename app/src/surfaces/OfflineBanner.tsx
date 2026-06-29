import { AnimatePresence, motion } from "motion/react";
import { WifiOff } from "lucide-react";
import { useOnline } from "../lib/online";

/** A thin banner shown while the machine is offline. History stays available
 *  from the local cache; new runs need the connection back. */
export function OfflineBanner() {
  const online = useOnline();
  return (
    <AnimatePresence>
      {!online && (
        <motion.div
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          exit={{ height: 0, opacity: 0 }}
          transition={{ duration: 0.18 }}
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
