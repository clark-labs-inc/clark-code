import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useIsNarrow } from "../../lib/responsive";
import { useModalFocus } from "../../lib/modalFocus";

/** Narrow windows retain full navigation in a dismissible, keyboard-accessible drawer. */
export function useSidebarDrawer(conversationId: string | null) {
  const narrow = useIsNarrow(768);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!narrow) return;
    const toggle = (event: globalThis.KeyboardEvent) => {
      if (event.key !== "\\" || !(event.metaKey || event.ctrlKey) || event.altKey) return;
      event.preventDefault();
      event.stopPropagation();
      setOpen((current) => !current);
    };
    window.addEventListener("keydown", toggle, true);
    return () => window.removeEventListener("keydown", toggle, true);
  }, [narrow]);
  const wasOpen = useRef(false);
  const ref = useModalFocus<HTMLElement>(narrow && open);
  useEffect(() => { setOpen(false); }, [conversationId, narrow]);
  useEffect(() => {
    if (narrow && wasOpen.current && !open) {
      requestAnimationFrame(() => document.querySelector<HTMLButtonElement>('button[aria-label="Expand sidebar"]')?.focus());
    }
    wasOpen.current = open;
  }, [narrow, open]);
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (!narrow || !open || event.defaultPrevented) return;
    if (event.key === "Escape" && (event.target as HTMLElement).closest('[role="menu"]')) return;
    if (event.key === "Escape") {
      event.preventDefault();
      setOpen(false);
    }
    if (event.key !== "Tab") return;
    event.stopPropagation();
    const items = Array.from(ref.current?.querySelectorAll<HTMLElement>(
      'button:not([disabled]), input:not([disabled]), [tabindex="0"]',
    ) ?? []).filter((element) => element.getClientRects().length > 0);
    const first = items[0];
    const last = items.at(-1);
    if (event.shiftKey && (document.activeElement === first || document.activeElement === ref.current)) {
      event.preventDefault(); last?.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault(); first?.focus();
    }
  };
  return { narrow, open, setOpen, ref, onKeyDown };
}
