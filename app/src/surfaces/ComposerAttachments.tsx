import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { FileText, X } from "lucide-react";
import type { PendingPaste } from "../lib/attachments";
import { DUR, EASE } from "../lib/motion";
import { useSessionStore } from "../store/sessionStore";

/** Compact, single-line attachment rail inspired by the mobile composer. */
export function AttachmentChips({
  pastes,
  onRemovePaste,
}: {
  pastes: PendingPaste[];
  onRemovePaste: (id: string) => void;
}) {
  const reduce = useReducedMotion();
  const attachments = useSessionStore((state) => state.attachments);
  const remove = useSessionStore((state) => state.removeAttachment);
  if (attachments.length === 0 && pastes.length === 0) return null;

  return (
    <div
      role="list"
      aria-label="Attachments"
      className="-mx-1 mb-1.5 overflow-x-auto px-1 pb-0.5 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
    >
      <div className="flex w-max gap-2">
        <AnimatePresence initial={false}>
          {attachments.map((attachment) => (
            <motion.div
              role="listitem"
              key={attachment.id}
              layout={!reduce}
              initial={reduce ? false : { opacity: 0, scale: 0.96, y: 3 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={reduce ? { opacity: 0, transition: { duration: 0 } } : { opacity: 0, scale: 0.96, y: 2 }}
              transition={{ duration: reduce ? 0 : DUR.base, ease: EASE.out }}
              className="group flex h-10 shrink-0 items-center gap-1.5 rounded-xl bg-bg-tertiary p-1 pr-1.5"
              title={attachment.filename}
            >
              {attachment.previewUrl ? (
                <img
                  src={attachment.previewUrl}
                  alt=""
                  className="size-8 rounded-lg object-cover"
                />
              ) : (
                <span className="grid size-8 place-items-center rounded-lg bg-bg-sunken text-ink-muted">
                  <FileText className="size-4" />
                </span>
              )}
              <span className="max-w-36 truncate text-xs font-medium text-ink-secondary">
                {attachment.filename}
              </span>
              <button
                type="button"
                onClick={() => remove(attachment.id)}
                aria-label={`Remove ${attachment.filename}`}
                className="grid size-5 place-items-center rounded-full bg-ink/10 text-ink-muted transition hover:bg-danger/20 hover:text-danger"
              >
                <X className="size-3" />
              </button>
            </motion.div>
          ))}
          {pastes.map((paste) => (
            <motion.div
              role="listitem"
              key={paste.id}
              layout={!reduce}
              initial={reduce ? false : { opacity: 0, scale: 0.96, y: 3 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={reduce ? { opacity: 0, transition: { duration: 0 } } : { opacity: 0, scale: 0.96, y: 2 }}
              transition={{ duration: reduce ? 0 : DUR.base, ease: EASE.out }}
              className="group flex h-10 shrink-0 items-center gap-1.5 rounded-xl bg-bg-tertiary p-1 pr-1.5"
              title={paste.placeholder}
            >
              <span className="grid size-8 place-items-center rounded-lg bg-bg-sunken text-ink-muted">
                <FileText className="size-4" />
              </span>
              <span className="max-w-44 truncate text-xs font-medium text-ink-secondary">
                {paste.placeholder.replace("[", "").replace("]", "")}
              </span>
              <button
                type="button"
                onClick={() => onRemovePaste(paste.id)}
                aria-label={`Remove ${paste.placeholder}`}
                className="grid size-5 place-items-center rounded-full bg-ink/10 text-ink-muted transition hover:bg-danger/20 hover:text-danger"
              >
                <X className="size-3" />
              </button>
            </motion.div>
          ))}
        </AnimatePresence>
      </div>
    </div>
  );
}
