import type { KeyboardEvent } from "react";

/** Native-style arrow/Home/End navigation for action menus; Tab remains available. */
export function handleMenuNavigation(event: KeyboardEvent<HTMLElement>) {
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) return;
  const items = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not([disabled])'));
  if (!items.length) return;
  event.preventDefault();
  const current = items.findIndex((item) => item === document.activeElement);
  const index = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1
    : current < 0 ? (event.key === "ArrowDown" ? 0 : items.length - 1)
    : (current + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
  items[index].focus();
}
