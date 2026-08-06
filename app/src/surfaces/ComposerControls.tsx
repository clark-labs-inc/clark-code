import { useEffect, useRef, useState, type RefObject } from "react";
import { Check, ChevronDown } from "lucide-react";

import { cn } from "../lib/cn";
import {
  CODING_MODELS,
  effectiveModelSettings,
  modelLabel,
} from "../lib/localAgent";
import { useSessionStore } from "../store/sessionStore";

function useOutsideClose(ref: RefObject<HTMLElement | null>, onClose: () => void) {
  const cb = useRef(onClose);
  cb.current = onClose;
  useEffect(() => {
    const handler = (event: Event) => {
      if (ref.current && !ref.current.contains(event.target as Node)) cb.current();
    };
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") cb.current();
    };
    document.addEventListener("mousedown", handler);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", handler);
      document.removeEventListener("keydown", onKey);
    };
  }, [ref]);
}

export function ModelPill() {
  const sessionId = useSessionStore((state) => state.session?.id ?? null);
  const model = useSessionStore(
    (state) => effectiveModelSettings(state.localSettings, state.chatModels, sessionId).model,
  );
  const update = useSessionStore((state) => state.updateModelSettings);
  const flashNotice = useSessionStore((state) => state.flashNotice);
  const busy = useSessionStore((state) =>
    Object.values(state.snapshot.runs).some(
      (run) => run.status === "running" || run.status === "queued" || run.status === "awaiting_input",
    ),
  );
  const [open, setOpen] = useState(false);
  const [switching, setSwitching] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useOutsideClose(ref, () => setOpen(false));

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => {
          if (busy) {
            flashNotice("Finish the current run before changing models.");
            return;
          }
          if (!switching) setOpen((value) => !value);
        }}
        disabled={busy || switching}
        aria-haspopup="menu"
        aria-expanded={open}
        title={busy ? "Finish the current run before changing models" : "Model"}
        className="flex min-h-8 items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium text-ink-secondary transition duration-200 ease-clark hover:bg-accent-subtle hover:text-accent"
      >
        {modelLabel(model)}
        <ChevronDown className="size-3 opacity-70" />
      </button>

      {open && (
        <div
          role="menu"
          className="popover-surface absolute bottom-full right-0 z-30 mb-2 max-h-[calc(100vh-7rem)] w-72 overflow-y-auto rounded-2xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle"
        >
          <div className="px-2.5 py-1.5 text-xs font-medium uppercase tracking-wide text-ink-faint">
            Model
          </div>
          {CODING_MODELS.map((candidate) => (
            <button
              key={candidate.id}
              type="button"
              role="menuitemradio"
              aria-checked={candidate.id === model}
              onClick={() => {
                if (busy || switching) return;
                setSwitching(true);
                setOpen(false);
                void update({ model: candidate.id }).finally(() => setSwitching(false));
              }}
              disabled={busy || switching}
              className={cn(
                "flex w-full items-start gap-2.5 rounded-xl px-2.5 py-2.5 text-left transition duration-200 ease-clark hover:bg-accent-subtle",
                candidate.id === model && "bg-accent-subtle",
              )}
            >
              <span className="min-w-0 flex-1">
                <span className="text-sm text-ink">{candidate.label}</span>
                <span className="block text-xs leading-snug text-ink-muted">
                  {candidate.hint}
                </span>
              </span>
              {candidate.id === model && <Check className="mt-0.5 size-4 shrink-0 text-accent" />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
