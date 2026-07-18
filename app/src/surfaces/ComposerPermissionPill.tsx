import { useEffect, useRef, useState } from "react";
import {
  Check,
  ChevronDown,
  ListChecks,
  Shield,
  ShieldAlert,
  ShieldCheck,
} from "lucide-react";
import { cn } from "../lib/cn";
import { PERMISSION_MODES, type PermissionMode } from "../lib/permissions";
import { useSessionStore } from "../store/sessionStore";

const MODE_ICON: Record<PermissionMode, typeof Shield> = {
  ask: Shield,
  auto: ShieldCheck,
  full: ShieldAlert,
  plan: ListChecks,
};

/** Codex-style approval policy selector. Full access is the default. */
export function ComposerPermissionPill() {
  const mode = useSessionStore((s) => s.permissionMode);
  const setMode = useSessionStore((s) => s.setPermissionMode);
  // Permission modes govern the LOCAL engine's gate; a Clark cloud session
  // runs every tool server-side in its own sandbox and never consults them.
  const isLocalTarget = useSessionStore((s) =>
    s.session ? s.session.provider === "local" : s.activeProvider === "local",
  );
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const close = (event: Event) => {
      if (ref.current && !ref.current.contains(event.target as Node)) setOpen(false);
    };
    const closeOnEscape = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, []);

  const info = PERMISSION_MODES.find((item) => item.id === mode) ?? PERMISSION_MODES[2];
  const Icon = MODE_ICON[mode];
  if (!isLocalTarget) return null;

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-haspopup="menu"
        aria-expanded={open}
        title="How Clark's actions are approved (Shift+Tab to cycle)"
        className={cn(
          "flex min-h-8 items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium transition duration-200 ease-clark hover:bg-accent-subtle",
          mode === "full"
            ? "text-warning"
            : mode === "plan"
              ? "bg-accent-subtle text-accent"
              : "text-ink-secondary",
        )}
      >
        <Icon className="size-3.5" />
        {info.label}
        <ChevronDown className="size-3 opacity-70" />
      </button>

      {open && (
        <div
          role="menu"
          className="popover-surface absolute bottom-full left-0 z-30 mb-2 w-72 rounded-2xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle"
        >
          <div className="px-2.5 py-1.5 text-xs font-medium uppercase tracking-wide text-ink-faint">
            How should Clark act?
          </div>
          {PERMISSION_MODES.map((item) => {
            const ItemIcon = MODE_ICON[item.id];
            return (
              <button
                key={item.id}
                type="button"
                role="menuitemradio"
                aria-checked={item.id === mode}
                onClick={() => {
                  setMode(item.id);
                  setOpen(false);
                }}
                className={cn(
                  "flex w-full items-start gap-2.5 rounded-xl px-2.5 py-2.5 text-left transition duration-200 ease-clark hover:bg-accent-subtle",
                  item.id === mode && "bg-accent-subtle",
                )}
              >
                <ItemIcon
                  className={cn(
                    "mt-0.5 size-4 shrink-0",
                    item.id === "full"
                      ? "text-warning"
                      : item.id === "plan"
                        ? "text-accent"
                        : "text-ink-muted",
                  )}
                />
                <span className="min-w-0 flex-1">
                  <span className="block text-sm text-ink">{item.label}</span>
                  <span className="block text-xs leading-snug text-ink-muted">
                    {item.description}
                  </span>
                </span>
                {item.id === mode && (
                  <Check className="mt-0.5 size-4 shrink-0 text-accent" />
                )}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
