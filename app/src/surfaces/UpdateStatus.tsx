import { useEffect } from "react";
import { productName } from "../product/productModule";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { CheckCircle2, X } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { DUR, EASE, REDUCED_EXIT, RISE, accessibleMotion } from "../lib/motion";
import { ProductMark } from "./ProductMark";

/** Full-window "applying update" overlay. The staged bundle is already on disk,
 *  so this is brief — its job is to make the relaunch feel intentional instead
 *  of the window vanishing with no explanation. */
function UpdateOverlay() {
  const applying = useSessionStore((s) => s.updateApplying);
  const version = useSessionStore((s) => s.update?.version);
  const reduce = useReducedMotion();
  return (
    <AnimatePresence initial={false}>
      {applying && (
        <m.div
          key="update-overlay"
          initial={reduce ? false : { opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={reduce ? REDUCED_EXIT : { opacity: 0 }}
          transition={{ duration: reduce ? 0 : DUR.base, ease: EASE.out }}
          className="fixed inset-0 z-[100] grid place-items-center bg-bg/80 backdrop-blur-sm"
        >
          <div
            className="popover-surface flex w-72 flex-col items-center gap-3 rounded-2xl border border-border-subtle bg-bg-elevated p-6 text-center shadow-xl"
          >
            <ProductMark size={40} className="rounded-xl" />
            <div>
              <div className="text-sm font-semibold text-ink">Updating {productName()}</div>
              <div className="mt-0.5 text-xs text-ink-muted">
                {version ? `Restarting into v${version}…` : "Restarting…"}
              </div>
            </div>
            <div className="relative h-1 w-full overflow-hidden rounded-full bg-bg-tertiary">
              <m.span
                className="absolute inset-y-0 left-0 w-1/3 rounded-full bg-accent"
                animate={reduce ? { x: 0 } : { x: ["-100%", "300%"] }}
                transition={reduce ? { duration: 0 } : { duration: 1.1, repeat: Infinity, ease: "easeInOut" }}
              />
            </div>
          </div>
        </m.div>
      )}
    </AnimatePresence>
  );
}

/** One-time confirmation shown after the app relaunches on a new version, so a
 *  silent restart is visibly accounted for. Auto-dismisses. */
function JustUpdatedToast() {
  const version = useSessionStore((s) => s.justUpdatedTo);
  const dismiss = useSessionStore((s) => s.dismissJustUpdated);
  const reduce = useReducedMotion();

  useEffect(() => {
    if (!version) return;
    const t = setTimeout(dismiss, 6000);
    return () => clearTimeout(t);
  }, [version, dismiss]);

  return (
    <AnimatePresence initial={false}>
      {version && (
        <div className="fixed bottom-4 left-1/2 z-[90] -translate-x-1/2">
          <m.div
            key="just-updated"
            {...accessibleMotion(RISE, reduce)}
          >
            <div
              role="status"
              className="popover-surface flex items-center gap-2.5 rounded-xl border border-border-subtle bg-bg-elevated px-3.5 py-2.5 shadow-lg"
            >
              <CheckCircle2 className="size-4 shrink-0 text-success" />
              <span className="text-sm text-ink">
                Updated to <span className="font-semibold">v{version}</span>
              </span>
              <button
                onClick={dismiss}
                aria-label="Dismiss"
                className="grid size-8 place-items-center rounded-md text-ink-faint transition hover:bg-bg-hover hover:text-ink"
              >
                <X className="size-3.5" />
              </button>
            </div>
          </m.div>
        </div>
      )}
    </AnimatePresence>
  );
}

/** Update lifecycle chrome mounted at the app root: the applying overlay and the
 *  post-restart confirmation toast. */
export function UpdateStatus() {
  return (
    <>
      <UpdateOverlay />
      <JustUpdatedToast />
    </>
  );
}
