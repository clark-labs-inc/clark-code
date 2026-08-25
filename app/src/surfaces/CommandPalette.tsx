import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import {
  Plus, SquareTerminal, Blocks, BookText, Sun, Moon, MessageSquare, CornerDownLeft,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { slashCommands } from "../lib/slashCommands";
import { paletteCommandPresentation } from "../lib/commandPalette";
import { fuzzyFilter } from "../lib/fuzzy";
import { projectName } from "../lib/localAgent";
import { cn } from "../lib/cn";
import { DIALOG, OVERLAY, accessibleMotion } from "../lib/motion";
import { useModalFocus } from "../lib/modalFocus";
import { productModule } from "../product/productModule";

interface PaletteItem {
  id: string;
  label: string;
  hint?: string;
  badge?: string;
  icon: typeof Plus;
  group: "Actions" | "Conversations";
  searchText: string;
  run: () => void;
}

const ICON: Record<string, typeof Plus> = {
  new: Plus,
  terminal: SquareTerminal,
  mcp: Blocks,
  memory: BookText,
};

/** Short palette labels; commands not listed here use their hint as the label. */
const LABELS: Record<string, string> = {
  new: "New session",
  terminal: "Toggle terminal",
  mcp: "Manage MCP servers",
  memory: "View project memory",
};

/** ⌘K command palette: fuzzy-search actions and conversations, keyboard-first. */
export function CommandPalette({
  dark,
  onToggleTheme,
}: {
  dark: boolean;
  onToggleTheme: () => void;
}) {
  const open = useSessionStore((s) => s.paletteOpen);
  const setOpen = useSessionStore((s) => s.setPaletteOpen);
  // Instant, no opacity fade under Reduced Motion — see Settings for why.
  const reduce = useReducedMotion();
  const session = useSessionStore((s) => s.session);
  const activeProvider = useSessionStore((s) => s.activeProvider);
  const activeRemote = useSessionStore((s) => s.activeRemote);
  const conversations = useSessionStore((s) => s.conversations);
  const openConversation = useSessionStore((s) => s.openConversation);
  const localTarget = session ? session.provider === "local" : activeProvider === "local";

  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const dialogRef = useModalFocus<HTMLDivElement>(open, inputRef);

  const items = useMemo<PaletteItem[]>(() => {
    const actions: PaletteItem[] = slashCommands()
      .filter(
        (c) =>
          (!c.needsSession || session)
          && (!c.localOnly || localTarget)
          && (c.name !== "terminal" || !activeRemote),
      )
      .map((c) => {
        const presentation = paletteCommandPresentation(c, LABELS[c.name]);
        return {
          id: `action:${c.name}`,
          label: presentation.label,
          hint: presentation.hint,
          badge: c.gatedWorkflow
            ? productModule().localAgent.workflowAccess?.badge ?? "Restricted"
            : undefined,
          icon: ICON[c.name] ?? Plus,
          group: "Actions",
          searchText: presentation.searchText,
          run: c.run ?? (() => {
            if (presentation.prefill !== null) {
              useSessionStore.getState().setComposerPrefill(presentation.prefill);
            }
          }),
        };
      });
    actions.push({
      id: "action:theme",
      label: dark ? "Switch to light theme" : "Switch to dark theme",
      icon: dark ? Sun : Moon,
      group: "Actions",
      searchText: `theme ${dark ? "light" : "dark"}`,
      run: onToggleTheme,
    });
    const convos: PaletteItem[] = conversations.map((c) => ({
      id: `convo:${c.id}`,
      label: c.title,
      hint: c.project ? projectName(c.project) : undefined,
      icon: MessageSquare,
      group: "Conversations",
      searchText: `${c.title} ${c.project ? projectName(c.project) : ""}`,
      run: () => void openConversation(c.id),
    }));
    return [...actions, ...convos];
  }, [
    session,
    localTarget,
    activeRemote,
    conversations,
    dark,
    onToggleTheme,
    openConversation,
  ]);

  const matches = useMemo(
    () => fuzzyFilter(items, query, (i) => i.searchText, 40).map((m) => m.item),
    [items, query],
  );

  // Reset on open; keep the active row in range as results change.
  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
    }
  }, [open]);
  useEffect(() => setActive(0), [query]);

  if (!open) return null;

  const run = (item?: PaletteItem) => {
    if (!item) return;
    setOpen(false);
    item.run();
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(a + 1, matches.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      run(matches[active]);
    } else if (e.key === "Escape") {
      e.preventDefault();
      setOpen(false);
    }
  };

  // Keep the active row scrolled into view.
  const setRowRef = (i: number) => (el: HTMLButtonElement | null) => {
    if (i === active && el) el.scrollIntoView({ block: "nearest" });
  };

  let lastGroup = "";

  return (
    <AnimatePresence>
      <m.div
        {...accessibleMotion(OVERLAY, reduce)}
        className="fixed inset-0 z-50 flex items-start justify-center bg-scrim-strong px-4 pt-[12vh]"
        onMouseDown={(e) => e.target === e.currentTarget && setOpen(false)}
      >
        <m.div
          ref={dialogRef}
          {...accessibleMotion(DIALOG, reduce)}
          role="dialog"
          aria-modal="true"
          aria-label="Command palette"
          className="popover-surface flex max-h-[70vh] w-full max-w-xl flex-col overflow-hidden rounded-2xl bg-bg-elevated shadow-lifted ring-1 ring-border-subtle"
        >
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKey}
            placeholder="Search actions and conversations…"
            aria-label="Search commands"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            className="composer-input w-full shrink-0 bg-bg-secondary/55 px-4 py-3.5 text-sm text-ink outline-none placeholder:text-ink-muted"
          />
          <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto p-1.5">
            {matches.length === 0 ? (
              <p className="px-3 py-6 text-center text-sm text-ink-faint">No matches.</p>
            ) : (
              matches.map((item, i) => {
                const showGroup = item.group !== lastGroup;
                lastGroup = item.group;
                const Icon = item.icon;
                return (
                  <div key={item.id}>
                    {showGroup && (
                      <div className="px-2.5 pb-1 pt-2 text-xs font-medium uppercase tracking-wide text-ink-faint">
                        {item.group}
                      </div>
                    )}
                    <button
                      ref={setRowRef(i)}
                      onMouseMove={() => setActive(i)}
                      onClick={() => run(item)}
                      className={cn(
                        "flex min-h-9 w-full items-center gap-2.5 rounded-xl px-2.5 py-2 text-left text-sm transition duration-base ease-agent",
                        i === active ? "bg-accent-subtle text-ink" : "text-ink-secondary",
                      )}
                    >
                      <Icon className="size-4 shrink-0 text-ink-muted" />
                      <span className="min-w-0 flex-1 truncate">{item.label}</span>
                      {item.hint && (
                        <span className="shrink-0 truncate text-xs text-ink-faint">{item.hint}</span>
                      )}
                      {item.badge && (
                        <span className="shrink-0 rounded-md bg-accent/10 px-1.5 py-0.5 text-xs font-medium text-accent">
                          {item.badge}
                        </span>
                      )}
                      {i === active && <CornerDownLeft className="size-3.5 shrink-0 text-ink-faint" />}
                    </button>
                  </div>
                );
              })
            )}
          </div>
        </m.div>
      </m.div>
    </AnimatePresence>
  );
}
