import { useEffect, useRef } from "react";
import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { SquareTerminal, X } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
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

export function TerminalPanel() {
  const open = useSessionStore((s) => s.terminalOpen);
  const setOpen = useSessionStore((s) => s.setTerminalOpen);
  const cwd = useSessionStore((s) => s.localSettings.cwd);
  const sessionId = useSessionStore((s) => s.session?.id);
  const hostRef = useRef<HTMLDivElement>(null);

  // Spin up a fresh PTY each time the drawer opens (or the project changes). The
  // shell streams over Tauri events into the xterm instance; everything is torn
  // down on close so no PTY is left running in the background.
  useEffect(() => {
    if (!open || !isTauri()) return;
    const host = hostRef.current;
    if (!host) return;

    const id = crypto.randomUUID();
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
    fit.fit();

    let disposed = false;
    const unlisten: Array<() => void> = [];

    void openTerminal(id, cwd || undefined, term.cols, term.rows).then(async () => {
      if (disposed) return;
      unlisten.push(await onTerminalData(id, (bytes) => term.write(bytes)));
      unlisten.push(
        await onTerminalExit(id, () =>
          term.write("\r\n\x1b[2m[process exited — reopen the terminal to start a new shell]\x1b[0m\r\n"),
        ),
      );
    });

    const dataSub = term.onData((d) => void writeTerminal(id, d));

    const ro = new ResizeObserver(() => {
      try {
        fit.fit();
        void resizeTerminal(id, term.cols, term.rows);
      } catch {
        /* host not laid out yet */
      }
    });
    ro.observe(host);

    // Re-theme when the app toggles light/dark.
    const mo = new MutationObserver(() => {
      term.options.theme = readTheme();
    });
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });

    term.focus();

    return () => {
      disposed = true;
      ro.disconnect();
      mo.disconnect();
      dataSub.dispose();
      for (const u of unlisten) u();
      void closeTerminal(id);
      term.dispose();
    };
  }, [open, cwd, sessionId]);

  if (!open) return null;

  return (
    <div className="flex h-72 shrink-0 flex-col border-t border-border bg-bg-sunken">
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border-subtle px-3 text-xs">
        <SquareTerminal className="size-3.5 text-ink-muted" />
        <span className="font-medium text-ink-secondary">Terminal</span>
        {cwd && <span className="truncate font-mono text-ink-faint">{projectName(cwd)}</span>}
        <button
          onClick={() => setOpen(false)}
          aria-label="Close terminal"
          className="ml-auto grid size-6 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink"
        >
          <X className="size-3.5" />
        </button>
      </div>
      {isTauri() ? (
        <div ref={hostRef} className="min-h-0 flex-1 overflow-hidden px-2 py-1" />
      ) : (
        <div className="grid flex-1 place-items-center px-4 text-center text-xs text-ink-faint">
          The terminal runs your shell on this machine — available in the desktop app.
        </div>
      )}
    </div>
  );
}
