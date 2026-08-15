import { useEffect, useRef, useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown } from "lucide-react";

import { cn } from "../lib/cn";
import {
  CODING_MODELS,
  effectiveModelSettings,
  modelLabel,
} from "../lib/localAgent";
import { useSessionStore } from "../store/sessionStore";

function useOutsideClose(
  triggerRef: RefObject<HTMLElement | null>,
  menuRef: RefObject<HTMLElement | null>,
  onClose: () => void,
) {
  const cb = useRef(onClose);
  cb.current = onClose;
  useEffect(() => {
    const handler = (event: Event) => {
      const target = event.target as Node;
      if (!triggerRef.current?.contains(target) && !menuRef.current?.contains(target)) cb.current();
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
  }, [menuRef, triggerRef]);
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
  const menuRef = useRef<HTMLDivElement>(null);
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const [menuPosition, setMenuPosition] = useState<{ left: number; bottom: number } | null>(null);
  useOutsideClose(ref, menuRef, () => setOpen(false));

  const positionMenu = () => {
    const rect = ref.current?.getBoundingClientRect();
    if (!rect) return;
    const menuWidth = Math.min(288, window.innerWidth - 24);
    setMenuPosition({
      left: Math.max(12, Math.min(rect.right - menuWidth, window.innerWidth - menuWidth - 12)),
      bottom: window.innerHeight - rect.top + 8,
    });
  };

  useEffect(() => {
    if (!open) return;
    positionMenu();
    const reposition = () => positionMenu();
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const selectedIndex = CODING_MODELS.findIndex((candidate) => candidate.id === model);
    const frame = requestAnimationFrame(() => itemRefs.current[selectedIndex]?.focus());
    return () => cancelAnimationFrame(frame);
  }, [model, open]);

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => {
          if (busy) {
            flashNotice("Finish the current run before changing models.");
            return;
          }
          if (switching) return;
          if (!open) positionMenu();
          setOpen((value) => !value);
        }}
        disabled={busy || switching}
        aria-haspopup="menu"
        aria-expanded={open}
        title={busy ? "Finish the current run before changing models" : "Model"}
        className="flex min-h-8 items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium text-ink-secondary transition duration-base ease-agent hover:bg-accent-subtle hover:text-accent"
      >
        {modelLabel(model)}
        <ChevronDown className="size-3 opacity-70" />
      </button>

      {open && menuPosition && typeof document !== "undefined" && createPortal(
        <div
          ref={menuRef}
          role="menu"
          aria-label="Model"
          onKeyDown={(event) => {
            const keys = ["ArrowDown", "ArrowUp", "Home", "End"];
            if (!keys.includes(event.key)) return;
            event.preventDefault();
            const current = itemRefs.current.indexOf(
              document.activeElement as HTMLButtonElement,
            );
            const last = CODING_MODELS.length - 1;
            const next =
              event.key === "Home"
                ? 0
                : event.key === "End"
                  ? last
                  : event.key === "ArrowDown"
                    ? (Math.max(current, -1) + 1) % CODING_MODELS.length
                    : current <= 0
                      ? last
                      : current - 1;
            itemRefs.current[next]?.focus();
          }}
          style={{ left: menuPosition.left, bottom: menuPosition.bottom }}
          className="popover-surface fixed z-[70] max-h-[calc(100vh-7rem)] w-72 max-w-[calc(100vw-1.5rem)] overflow-y-auto rounded-2xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle"
        >
          <div className="px-2.5 py-1.5 text-xs font-medium uppercase tracking-wide text-ink-faint">
            Model
          </div>
          {CODING_MODELS.map((candidate, index) => (
            <button
              key={candidate.id}
              ref={(node) => {
                itemRefs.current[index] = node;
              }}
              type="button"
              role="menuitemradio"
              aria-checked={candidate.id === model}
              tabIndex={candidate.id === model ? 0 : -1}
              onClick={() => {
                if (busy || switching) return;
                setSwitching(true);
                setOpen(false);
                void update({ model: candidate.id }).finally(() => setSwitching(false));
              }}
              disabled={busy || switching}
              className={cn(
                "flex w-full items-start gap-2.5 rounded-xl px-2.5 py-2.5 text-left transition duration-base ease-agent hover:bg-accent-subtle",
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
        </div>,
        document.body,
      )}
    </div>
  );
}

export function QuickChatModelLabel() {
  return (
    <span
      aria-label="Quick Chat uses the Free tier"
      title="Quick Chat always uses the Free tier"
      className="flex min-h-8 items-center rounded-lg px-2.5 py-1 text-xs font-medium text-ink-secondary"
    >
      Free
    </span>
  );
}
