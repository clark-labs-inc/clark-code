import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  Check,
  ChevronDown,
  Shield,
  ShieldAlert,
  ShieldCheck,
} from "lucide-react";
import { cn } from "../lib/cn";
import { APPROVAL_POLICIES, type ApprovalPolicy } from "../lib/permissions";
import { effectiveApprovalPolicy } from "../store/sessionStore.runtime";
import { useSessionStore } from "../store/sessionStore";

const MODE_ICON: Record<ApprovalPolicy, typeof Shield> = {
  ask: Shield,
  auto: ShieldCheck,
  full: ShieldAlert,
};

/** Approval policy selector. Sandboxed "Approve for me" is the default. */
export function ComposerPermissionPill() {
  const session = useSessionStore((s) => s.session);
  const globalDefault = useSessionStore((s) => s.approvalPolicy);
  const approvalPolicies = useSessionStore((s) => s.approvalPolicies);
  // The pill shows THIS chat's level — its own override when it has one, else
  // the account default — so switching chats never displays a sibling's mode.
  const mode = effectiveApprovalPolicy(globalDefault, approvalPolicies, session?.id);
  const setMode = useSessionStore((s) => s.setApprovalPolicy);
  // Permission modes govern the LOCAL engine's gate; a Clark cloud session
  // runs every tool server-side in its own sandbox and never consults them.
  const isLocalTarget = useSessionStore((s) =>
    s.session ? s.session.provider === "local" : s.activeProvider === "local",
  );
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const [menuPosition, setMenuPosition] = useState<{ left: number; bottom: number } | null>(null);

  const positionMenu = () => {
    const rect = ref.current?.getBoundingClientRect();
    if (!rect) return;
    const menuWidth = Math.min(288, window.innerWidth - 24);
    setMenuPosition({
      left: Math.max(12, Math.min(rect.left, window.innerWidth - menuWidth - 12)),
      bottom: window.innerHeight - rect.top + 8,
    });
  };

  useEffect(() => {
    const close = (event: Event) => {
      const target = event.target as Node;
      if (!ref.current?.contains(target) && !menuRef.current?.contains(target)) setOpen(false);
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
    const selectedIndex = APPROVAL_POLICIES.findIndex((item) => item.id === mode);
    const frame = requestAnimationFrame(() => itemRefs.current[selectedIndex]?.focus());
    return () => cancelAnimationFrame(frame);
  }, [mode, open]);

  const info = APPROVAL_POLICIES.find((item) => item.id === mode) ?? APPROVAL_POLICIES[1];
  const Icon = MODE_ICON[mode];
  if (!isLocalTarget) return null;

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => {
          if (!open) positionMenu();
          setOpen((value) => !value);
        }}
        aria-haspopup="menu"
        aria-expanded={open}
        title="How Clark's actions are approved (Shift+Tab to cycle)"
        className={cn(
          "flex min-h-8 items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium transition duration-200 ease-clark hover:bg-accent-subtle",
          mode === "full" ? "text-warning" : "text-ink-secondary",
        )}
      >
        <Icon className="size-3.5" />
        {info.label}
        <ChevronDown className="size-3 opacity-70" />
      </button>

      {open && menuPosition && typeof document !== "undefined" && createPortal(
        <div
          ref={menuRef}
          role="menu"
          aria-label="Approval mode"
          onKeyDown={(event) => {
            const keys = ["ArrowDown", "ArrowUp", "Home", "End"];
            if (!keys.includes(event.key)) return;
            event.preventDefault();
            const current = itemRefs.current.indexOf(
              document.activeElement as HTMLButtonElement,
            );
            const last = APPROVAL_POLICIES.length - 1;
            const next =
              event.key === "Home"
                ? 0
                : event.key === "End"
                  ? last
                  : event.key === "ArrowDown"
                    ? (Math.max(current, -1) + 1) % APPROVAL_POLICIES.length
                    : current <= 0
                      ? last
                      : current - 1;
            itemRefs.current[next]?.focus();
          }}
          style={{ left: menuPosition.left, bottom: menuPosition.bottom }}
          className="popover-surface fixed z-[70] w-72 max-w-[calc(100vw-1.5rem)] rounded-xl bg-bg-elevated p-1.5 shadow-lifted ring-1 ring-border-subtle"
        >
          <div className="px-2.5 pb-1.5 pt-1 text-xs font-semibold text-ink-muted">
            Approval mode
          </div>
          {APPROVAL_POLICIES.map((item, index) => {
            const ItemIcon = MODE_ICON[item.id];
            return (
              <button
                key={item.id}
                ref={(node) => {
                  itemRefs.current[index] = node;
                }}
                type="button"
                role="menuitemradio"
                aria-checked={item.id === mode}
                tabIndex={item.id === mode ? 0 : -1}
                onClick={() => {
                  setMode(item.id);
                  setOpen(false);
                }}
                className={cn(
                  "flex w-full items-start gap-2.5 rounded-lg px-2.5 py-2 text-left transition duration-200 ease-clark hover:bg-accent-subtle",
                  item.id === mode && "bg-accent-subtle",
                )}
              >
                <ItemIcon
                  className={cn(
                    "mt-0.5 size-3.5 shrink-0",
                    item.id === "full" ? "text-warning" : "text-ink-muted",
                  )}
                />
                <span className="min-w-0 flex-1">
                  <span className="block text-sm font-medium leading-5 text-ink">
                    {item.label}
                  </span>
                  <span className="block text-xs leading-4 text-ink-muted">
                    {item.description}
                  </span>
                </span>
                {item.id === mode && (
                  <Check className="mt-1 size-3.5 shrink-0 text-accent" />
                )}
              </button>
            );
          })}
        </div>,
        document.body,
      )}
    </div>
  );
}
