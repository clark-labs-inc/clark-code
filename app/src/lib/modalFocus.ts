import { useLayoutEffect, useRef, type RefObject } from "react";

export interface ModalFocusTarget {
  readonly isConnected: boolean;
  focus(options?: FocusOptions): void;
}

/** Return focus only when the opener still belongs to the current document. */
export function restoreModalFocus(target: ModalFocusTarget | null): void {
  if (target?.isConnected) target.focus({ preventScroll: true });
}

let focusTrackerSubscribers = 0;
let lastPointerTarget: HTMLElement | null = null;

function rememberPointerTarget(event: PointerEvent): void {
  lastPointerTarget = event.target instanceof HTMLElement ? event.target : null;
}

function subscribeToModalTriggers(): () => void {
  focusTrackerSubscribers += 1;
  if (focusTrackerSubscribers === 1) {
    document.addEventListener("pointerdown", rememberPointerTarget, true);
  }
  return () => {
    focusTrackerSubscribers -= 1;
    if (focusTrackerSubscribers === 0) {
      document.removeEventListener("pointerdown", rememberPointerTarget, true);
      lastPointerTarget = null;
    }
  };
}

function currentFocusReturnTarget(): HTMLElement | null {
  const active = document.activeElement;
  if (
    active instanceof HTMLElement
    && active !== document.body
    && active !== document.documentElement
  ) {
    return active;
  }
  return lastPointerTarget?.isConnected ? lastPointerTarget : null;
}

/**
 * Own focus while a modal is open and return it to the exact opener on close.
 *
 * Layout effects run after the modal DOM changes but before paint, preventing
 * a one-frame focus jump to `<body>` during reduced-motion transitions.
 */
export function useModalFocus<T extends HTMLElement>(
  open: boolean,
  initialFocusRef?: RefObject<HTMLElement | null>,
): RefObject<T | null> {
  const dialogRef = useRef<T>(null);

  // CommandPalette stays mounted while closed, so this shared capture listener
  // also remembers pointer-openers for lazy modals (Settings/MCP/SSH). This is
  // required in WKWebView, where clicking a button does not necessarily focus it.
  useLayoutEffect(subscribeToModalTriggers, []);

  useLayoutEffect(() => {
    if (!open) return;
    const returnFocus = currentFocusReturnTarget();
    (initialFocusRef?.current ?? dialogRef.current)?.focus({ preventScroll: true });
    // Cleanup runs both when `open` becomes false and when a parent conditionally
    // unmounts the modal, covering Settings' lazy shell lifecycle.
    return () => restoreModalFocus(returnFocus);
  }, [initialFocusRef, open]);

  return dialogRef;
}
