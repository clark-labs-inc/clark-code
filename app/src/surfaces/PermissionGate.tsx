import { motion, useReducedMotion } from "motion/react";
import { ShieldQuestion } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { cn } from "../lib/cn";
import type { PermissionOptionKind } from "../core-bridge/types";

const OPTION_STYLE: Record<PermissionOptionKind, string> = {
  allow_once: "bg-accent text-on-accent hover:bg-accent-hover",
  allow_always: "border border-accent/40 text-ink hover:bg-bg-hover",
  reject_once: "bg-bg-tertiary text-ink-secondary hover:bg-bg-hover",
  reject_always: "bg-danger/12 text-danger hover:bg-danger/20",
};

/** Inline human-in-the-loop gate — appears in the conversation flow so the user
 *  always sees, in context, exactly what the agent is asking to do. */
export function PermissionGate() {
  const req = useSessionStore((s) => s.snapshot.pending_permission);
  const resolve = useSessionStore((s) => s.resolvePermission);
  const reduce = useReducedMotion();
  if (!req) return null;

  return (
    <motion.div
      initial={reduce ? false : { opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      role="alertdialog"
      aria-label="Permission required"
      className="rounded-lg border border-warning/40 bg-warning/8 p-3.5"
    >
      <div className="mb-2 flex items-center gap-2 text-sm font-medium text-ink">
        <ShieldQuestion className="size-4 text-warning" />
        Permission required
      </div>
      <p className="mb-3 text-sm text-ink-secondary">{req.title}</p>
      <div className="flex flex-wrap gap-2">
        {req.options.map((opt) => (
          <button
            key={opt.id}
            onClick={() => void resolve(opt.id)}
            className={cn(
              "rounded-lg px-3 py-1.5 text-sm font-medium transition",
              OPTION_STYLE[opt.kind],
            )}
          >
            {opt.label}
          </button>
        ))}
      </div>
    </motion.div>
  );
}
