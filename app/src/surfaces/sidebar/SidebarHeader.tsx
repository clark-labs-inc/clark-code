import { useEffect, useState, type RefObject } from "react";
import { PanelLeft, Search, SquarePen, X } from "lucide-react";
import { useSessionStore } from "../../store/sessionStore";
import { productName } from "../../product/productModule";

export function SidebarHeader({ rail, onToggle, filter, onFilter, searchRef, searchOpen, onSearchOpenChange }: {
  rail: boolean;
  onToggle: () => void;
  filter: string;
  onFilter: (value: string) => void;
  searchRef: RefObject<HTMLInputElement | null>;
  searchOpen: boolean;
  onSearchOpenChange: (open: boolean) => void;
}) {
  const quickChat = useSessionStore((s) => s.startQuickChat);
  const [starting, setStarting] = useState(false);
  const searching = searchOpen || Boolean(filter);
  useEffect(() => {
    if (searching && !rail) searchRef.current?.focus();
  }, [searching, rail, searchRef]);
  const openSearch = () => { onSearchOpenChange(true); if (rail) onToggle(); };
  const iconButton = "grid size-8 shrink-0 place-items-center rounded-lg text-ink-muted hover:bg-bg-hover hover:text-ink";
  return (
    <div className={rail ? "flex flex-col items-center gap-1" : "shrink-0 px-3 pb-4"}>
      <div className={rail ? "contents" : "flex min-h-12 items-center gap-1"}>
        {!rail && <span className="min-w-0 flex-1 truncate px-1 text-base font-semibold text-ink">{productName()}</span>}
        {!rail && <button type="button" onClick={openSearch} aria-label="Search projects and chats" title="Search projects and chats"
          aria-expanded={searching} className={iconButton}><Search className="size-4" /></button>}
        <button type="button" onClick={onToggle} aria-label={rail ? "Expand sidebar" : "Collapse sidebar"}
          title={rail ? "Expand sidebar" : "Collapse sidebar"} className={iconButton}>
          <PanelLeft className="size-4" />
        </button>
      </div>
      <button type="button" disabled={starting} onClick={async () => {
        if (starting) return;
        setStarting(true);
        try {
          useSessionStore.getState().setConversationSelection(new Set());
          await quickChat();
          if (!useSessionStore.getState().error) requestAnimationFrame(() => document.querySelector<HTMLTextAreaElement>("textarea.composer-input")?.focus());
        } finally { setStarting(false); }
      }} aria-label="New chat" title="New chat — no project required"
        className={rail ? iconButton : "flex min-h-10 w-full items-center gap-2.5 rounded-lg px-2 text-sm text-ink-secondary hover:bg-bg-hover hover:text-ink disabled:opacity-50"}>
        <SquarePen className="size-4" />{!rail && (starting ? "Starting…" : "New chat")}
      </button>
      {rail && <button type="button" onClick={openSearch} aria-label="Search projects and chats" title="Search projects and chats" className={iconButton}><Search className="size-4" /></button>}
      {!rail && searching && <div className="mt-2 flex min-h-9 items-center gap-2 rounded-lg bg-bg px-2.5 ring-1 ring-border-subtle focus-within:ring-accent">
        <Search className="size-3.5 shrink-0 text-ink-muted" />
        <input ref={searchRef} value={filter} onChange={(e) => onFilter(e.target.value)}
          placeholder="Search projects and chats" aria-label="Search projects and chats" autoCorrect="off" autoCapitalize="off" spellCheck={false}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation(); onFilter(""); onSearchOpenChange(false);
              document.querySelector<HTMLButtonElement>('button[aria-label="Search projects and chats"]')?.focus();
            }
          }}
          className="min-w-0 flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-ink-muted" />
        <button type="button" onClick={() => {
          onFilter(""); onSearchOpenChange(false);
          requestAnimationFrame(() => document.querySelector<HTMLButtonElement>('button[aria-label="Search projects and chats"]')?.focus());
        }} aria-label="Close search" className={iconButton}><X className="size-3.5" /></button>
      </div>}
    </div>
  );
}
