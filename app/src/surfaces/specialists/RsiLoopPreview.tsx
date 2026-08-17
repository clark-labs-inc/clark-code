import { useEffect, useState } from "react";
import { ArrowUp, Plus, Sparkles } from "lucide-react";

import { specialistConversationPresentation } from "../../lib/specialistPresentation";
import { RsiLoopPulseCard } from "./RsiLoopPulse";

export function RsiLoopPreview() {
  const [paused, setPaused] = useState(false);
  const presentation = specialistConversationPresentation("rsi");

  useEffect(() => {
    const root = document.documentElement;
    const wasDark = root.classList.contains("dark");
    root.classList.add("dark");
    return () => {
      if (!wasDark) root.classList.remove("dark");
    };
  }, []);

  if (!presentation) return null;

  return (
    <main className="min-h-screen bg-bg px-6 py-14 text-ink">
      <div className="mx-auto w-full max-w-4xl">
        <div className="mb-4 flex items-start gap-3">
          <span className="mt-0.5 grid size-9 shrink-0 place-items-center rounded-full border border-accent/50 bg-accent-soft text-accent">
            <Sparkles className="size-4" aria-hidden="true" />
          </span>
          <div>
            <div className="flex items-baseline gap-2">
              <strong className="text-sm text-ink">Clark</strong>
              <span className="text-xs text-ink-faint">now</span>
            </div>
            <p className="mt-1 max-w-3xl text-base leading-7 text-ink-secondary">
              I’m improving planning reliability now. I’ll keep a change only when the result gets better and every safety guardrail still passes.
            </p>
          </div>
        </div>

        <div className="ml-12">
          {paused && (
            <div role="status" className="mb-2 text-xs font-medium text-warning">
              Paused safely at the current checkpoint.
            </div>
          )}
          <RsiLoopPulseCard
            presentation={presentation}
            onPause={paused ? undefined : () => setPaused(true)}
          />
        </div>

        <div className="ml-12 mt-5 rounded-2xl border border-border bg-bg-elevated p-3">
          <label htmlFor="rsi-preview-message" className="sr-only">Message Clark about this run</label>
          <textarea
            id="rsi-preview-message"
            rows={2}
            placeholder="Message Clark about this run…"
            className="w-full resize-none bg-transparent px-2 py-1 text-sm text-ink outline-none placeholder:text-ink-faint"
          />
          <div className="mt-1 flex items-center justify-between">
            <button
              type="button"
              aria-label="Add attachment"
              className="grid size-8 place-items-center rounded-full bg-bg-secondary text-ink-muted"
            >
              <Plus className="size-4" />
            </button>
            <button
              type="button"
              aria-label="Send message"
              className="grid size-8 place-items-center rounded-full bg-accent text-on-accent"
            >
              <ArrowUp className="size-4" />
            </button>
          </div>
        </div>
      </div>
    </main>
  );
}
