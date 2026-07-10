import { useEffect, useRef, useState } from "react";
import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { SquareTerminal, X, Plus } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
import { cn } from "../lib/cn";
import {
  isTauri,
  openTerminal,
  writeTerminal,
  resizeTerminal,
  closeTerminal,
  onTerminalData,
  onTerminalExit,
} from "../lib/terminal";

const MONO = '"JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace';

/** Map the app's CSS theme variables onto an xterm color theme so the terminal
 *  matches the surrounding UI in both light and dark modes. */
function readTheme() {
  const cs = getComputedStyle(document.documentElement);
  const v = (name: string, fb: string) => cs.getPropertyValue(name).trim() || fb;
  const dark = document.documentElement.classList.contains("dark");
  return {
    background: v("--color-bg-sunken", dark ? "#0a0a0a" : "#f0f0f2"),
    foreground: v("--color-ink", dark ? "#f4f4f3" : "#14141a"),
    cursor: v("--color-ink", dark ? "#f4f4f3" : "#14141a"),
    cursorAccent: v("--color-bg-sunken", dark ? "#0a0a0a" : "#f0f0f2"),
    selectionBackground: dark ? "rgba(255,255,255,0.20)" : "rgba(0,0,0,0.16)",
  };
}

type TermTab = { id: string; n: number };

/** One live terminal: its own xterm instance + PTY, created on mount and torn
 *  down on unmount (closing the tab). Kept mounted while inactive — only its
 *  host is hidden by the parent — so switching tabs preserves scrollback and
 *  any running process. `active` drives a re-fit + focus when the tab is shown
 *  again (a hidden host has zero size, so it couldn't fit while backgrounded). */
function TerminalInstance({ id, cwd, active }: { id: string; cwd?: string; active: boolean }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<XTerm | null>(null);
  const fitRef = useRef<FitAddon | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const term = new XTerm({
      fontFamily: MONO,
      fontSize: 12.5,
      lineHeight: 1.2,
      cursorBlink: true,
      scrollback: 5000,
      theme: readTheme(),
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    termRef.current = term;
    fitRef.current = fit;
    try {
      fit.fit();
    } catch {
      /* host not laid out yet — the active-effect re-fits */
    }

    let disposed = false;
    const unlisten: Array<() => void> = [];

    void openTerminal(id, cwd || undefined, term.cols, term.rows).then(async () => {
      if (disposed) return;
      unlisten.push(await onTerminalData(id, (bytes) => term.write(bytes)));
      unlisten.push(
        await onTerminalExit(id, () =>
          term.write("\r\n\x1b[2m[process exited — close this tab or open a new one]\x1b[0m\r\n"),
        ),
      );
    });

    const dataSub = term.onData((d) => void writeTerminal(id, d));

    const ro = new ResizeObserver(() => {
      try {
        fit.fit();
        void resizeTerminal(id, term.cols, term.rows);
      } catch {
        /* host hidden or not laid out */
      }
    });
    ro.observe(host);

    const mo = new MutationObserver(() => {
      term.options.theme = readTheme();
    });
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });

    return () => {
      disposed = true;
      ro.disconnect();
      mo.disconnect();
      dataSub.dispose();
      for (const u of unlisten) u();
      void closeTerminal(id);
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
    // Created once per tab; `id`/`cwd` are captured at mount and never change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!active) return;
    // Defer to after the host is visible (display flips this same frame).
    const t = window.setTimeout(() => {
      try {
        fitRef.current?.fit();
        const term = termRef.current;
        if (term) {
          void resizeTerminal(id, term.cols, term.rows);
          term.focus();
        }
      } catch {
        /* ignore */
      }
    }, 0);
    return () => window.clearTimeout(t);
  }, [active, id]);

  return <div ref={hostRef} className="h-full w-full overflow-hidden px-2 py-1" />;
}

export function TerminalPanel() {
  const open = useSessionStore((s) => s.terminalOpen);
  const setOpen = useSessionStore((s) => s.setTerminalOpen);
  const cwd = useSessionStore((s) => s.localSettings.cwd);

  // Monotonic label counter so tab names stay stable as tabs open/close.
  const counter = useRef(0);
  const makeTab = (): TermTab => ({ id: crypto.randomUUID(), n: ++counter.current });
  const [tabs, setTabs] = useState<TermTab[]>(() => [makeTab()]);
  const [activeId, setActiveId] = useState<string>(() => tabs[0].id);
  const active = tabs.some((t) => t.id === activeId) ? activeId : tabs[0]?.id;

  const addTab = () => {
    const t = makeTab();
    setTabs((prev) => [...prev, t]);
    setActiveId(t.id);
  };

  const closeTab = (id: string) => {
    const idx = tabs.findIndex((t) => t.id === id);
    const next = tabs.filter((t) => t.id !== id);
    if (next.length === 0) {
      // Closing the last tab closes the whole drawer (unmounts every instance).
      setOpen(false);
      return;
    }
    setTabs(next);
    if (id === active) setActiveId(next[Math.min(idx, next.length - 1)].id);
  };

  if (!open) return null;

  return (
    <div className="flex h-72 shrink-0 flex-col border-t border-border bg-bg-sunken">
      <div className="flex h-8 shrink-0 items-center gap-1 border-b border-border-subtle pl-2 pr-1 text-xs">
        <SquareTerminal className="size-3.5 shrink-0 text-ink-muted" />

        {isTauri() ? (
          <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
            {tabs.map((t) => (
              <div
                key={t.id}
                role="tab"
                aria-selected={t.id === active}
                onClick={() => setActiveId(t.id)}
                className={cn(
                  "group/tab flex shrink-0 cursor-pointer items-center gap-1.5 rounded-md py-1 pl-2 pr-1 transition",
                  t.id === active
                    ? "bg-accent-soft text-accent"
                    : "text-ink-muted hover:bg-accent-subtle hover:text-accent",
                )}
              >
                <span className="font-medium">Terminal {t.n}</span>
                <span
                  role="button"
                  aria-label="Close tab"
                  onClick={(e) => {
                    e.stopPropagation();
                    closeTab(t.id);
                  }}
                  className="grid size-4 place-items-center rounded text-ink-faint opacity-0 transition hover:text-ink group-hover/tab:opacity-100"
                >
                  <X className="size-3" />
                </span>
              </div>
            ))}
            <button
              onClick={addTab}
              aria-label="New terminal tab"
              className="grid size-6 shrink-0 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink"
            >
              <Plus className="size-3.5" />
            </button>
          </div>
        ) : (
          <span className="font-medium text-ink-secondary">Terminal</span>
        )}

        {cwd && (
          <span className="ml-1 hidden max-w-[12rem] truncate font-mono text-ink-faint sm:inline">
            {projectName(cwd)}
          </span>
        )}
        <button
          onClick={() => setOpen(false)}
          aria-label="Close terminal"
          className="ml-1 grid size-6 shrink-0 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink"
        >
          <X className="size-3.5" />
        </button>
      </div>

      {isTauri() ? (
        <div className="relative min-h-0 flex-1">
          {tabs.map((t) => (
            <div key={t.id} className={cn("absolute inset-0", t.id !== active && "hidden")}>
              <TerminalInstance id={t.id} cwd={cwd || undefined} active={t.id === active} />
            </div>
          ))}
        </div>
      ) : (
        <div className="grid flex-1 place-items-center px-4 text-center text-xs text-ink-faint">
          The terminal runs your shell on this machine — available in the desktop app.
        </div>
      )}
    </div>
  );
}
