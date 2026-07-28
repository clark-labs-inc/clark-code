import { useEffect, useRef, useState, type RefObject } from "react";
import { Check, ChevronDown } from "lucide-react";

import { cn } from "../lib/cn";
import {
  CODING_MODELS,
  effectiveModelSettings,
  modelLabel,
  normalizeReasoningEffort,
  reasoningEffortsForModel,
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

export function ModelPriceCue({ tier }: { tier: 0 | 1 | 2 | 3 | 4 | 5 }) {
  if (tier === 0) return null;
  return (
    <span
      aria-hidden="true"
      className="text-[8px] font-normal leading-none tracking-[-0.08em] text-ink-faint opacity-60"
    >
      {"$".repeat(tier)}
    </span>
  );
}

export function ModelPill() {
  const sessionId = useSessionStore((state) => state.session?.id ?? null);
  const model = useSessionStore(
    (state) => effectiveModelSettings(state.localSettings, state.chatModels, sessionId).model,
  );
  const effort = useSessionStore(
    (state) =>
      effectiveModelSettings(state.localSettings, state.chatModels, sessionId).reasoningEffort,
  );
  const update = useSessionStore((state) => state.updateModelSettings);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useOutsideClose(ref, () => setOpen(false));

  const reasoningEfforts = reasoningEffortsForModel(model);
  const effortLabel = reasoningEfforts.find((candidate) => candidate.id === effort)?.label;

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-haspopup="menu"
        aria-expanded={open}
        title="Model & reasoning effort"
        className="flex min-h-8 items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium text-ink-secondary transition duration-200 ease-clark hover:bg-accent-subtle hover:text-accent"
      >
        {modelLabel(model)}
        {effort && effortLabel && <span className="text-ink-faint">· {effortLabel}</span>}
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
                void update({
                  model: candidate.id,
                  reasoningEffort: normalizeReasoningEffort(candidate.id, effort),
                });
                setOpen(false);
              }}
              className={cn(
                "flex w-full items-start gap-2.5 rounded-xl px-2.5 py-2.5 text-left transition duration-200 ease-clark hover:bg-accent-subtle",
                candidate.id === model && "bg-accent-subtle",
              )}
            >
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-1 text-sm text-ink">
                  <span>{candidate.label}</span>
                  <ModelPriceCue tier={candidate.priceTier} />
                </span>
                <span className="block text-xs leading-snug text-ink-muted">
                  {candidate.hint}
                </span>
              </span>
              {candidate.id === model && <Check className="mt-0.5 size-4 shrink-0 text-accent" />}
            </button>
          ))}

          <div className="mx-1.5 my-1 border-t border-border-subtle" />
          <div className="px-2.5 py-1.5 text-xs font-medium uppercase tracking-wide text-ink-faint">
            Reasoning effort
          </div>
          {reasoningEfforts.length > 0 ? (
            <div className="flex gap-1 px-2.5 pb-2">
              {reasoningEfforts.map((candidate) => (
                <button
                  key={candidate.id}
                  type="button"
                  role="menuitemradio"
                  aria-checked={candidate.id === effort}
                  onClick={() => {
                    void update({ reasoningEffort: candidate.id });
                    setOpen(false);
                  }}
                  className={cn(
                    "min-h-8 flex-1 rounded-lg px-1 py-1 text-xs font-medium transition duration-200 ease-clark",
                    candidate.id === effort
                      ? "bg-accent text-on-accent"
                      : "bg-bg-tertiary text-ink-secondary hover:bg-bg-hover",
                  )}
                >
                  {candidate.label}
                </button>
              ))}
            </div>
          ) : (
            <p className="px-2.5 pb-2 text-xs leading-snug text-ink-muted">
              This model controls its reasoning effort automatically.
            </p>
          )}
          <p className="px-2.5 pb-1.5 text-xs leading-snug text-ink-faint">
            {reasoningEfforts.some((candidate) => candidate.id === "")
              ? "Auto uses this model's provider default. "
              : "Only levels supported by this model are shown. "}
            Applies from the next message — the conversation keeps its context.
          </p>
        </div>
      )}
    </div>
  );
}
