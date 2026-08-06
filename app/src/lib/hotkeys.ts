import { useEffect, useRef } from "react";

/** One keyboard binding. `mod` is ⌘ on macOS / Ctrl elsewhere. */
export interface Hotkey {
  key: string;
  mod?: boolean;
  shift?: boolean;
  run: () => void;
  /** Fire even while a text input/textarea is focused (e.g. ⌘↵ to send). */
  allowInInput?: boolean;
}

function isTyping(el: EventTarget | null): boolean {
  const node = el as HTMLElement | null;
  if (!node) return false;
  return (
    node.tagName === "INPUT" ||
    node.tagName === "TEXTAREA" ||
    node.isContentEditable === true
  );
}

/** Register global keyboard shortcuts. Bindings are read through a ref, so the
 *  listener is attached once and callers don't need to memoize the array. */
export function useHotkeys(bindings: Hotkey[]): void {
  const ref = useRef(bindings);
  ref.current = bindings;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      for (const b of ref.current) {
        if (e.key.toLowerCase() !== b.key.toLowerCase()) continue;
        if (!!b.mod !== mod) continue;
        if (!!b.shift !== e.shiftKey) continue;
        if (isTyping(e.target) && !b.allowInInput) continue;
        e.preventDefault();
        b.run();
        return;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
