import { lazy, Suspense, useRef } from "react";
import { useSessionStore } from "./store/sessionStore";
import { useTheme } from "./lib/useTheme";
import { useHotkeys } from "./lib/hotkeys";
import { TopBar } from "./surfaces/TopBar";
import { Sidebar } from "./surfaces/Sidebar";
import { StartCard } from "./surfaces/StartCard";
import { OpeningScreen } from "./surfaces/OpeningScreen";
import { Conversation } from "./surfaces/Conversation";
import { Composer } from "./surfaces/Composer";
import { CreditBanner } from "./surfaces/CreditBanner";
import { OfflineBanner } from "./surfaces/OfflineBanner";
import { CommandPalette } from "./surfaces/CommandPalette";
import { MobileRemoteAgent } from "./surfaces/MobileRemoteAgent";

const TerminalPanel = lazy(() =>
  import("./surfaces/TerminalPanel").then((module) => ({ default: module.TerminalPanel })),
);
const McpSettings = lazy(() =>
  import("./surfaces/McpSettings").then((module) => ({ default: module.McpSettings })),
);
const SshSettings = lazy(() =>
  import("./surfaces/SshSettings").then((module) => ({ default: module.SshSettings })),
);
const Settings = lazy(() =>
  import("./surfaces/Settings").then((module) => ({ default: module.Settings })),
);

export default function AuthenticatedWorkspace() {
  const session = useSessionStore((state) => state.session);
  const openingScreen = useSessionStore(
    (state) => state.opening !== null && state.opening.kind !== "peek",
  );
  const terminalOpen = useSessionStore((state) => state.terminalOpen);
  const mcpOpen = useSessionStore((state) => state.mcpOpen);
  const sshOpen = useSessionStore((state) => state.sshOpen);
  const settingsOpen = useSessionStore((state) => state.settingsOpen);
  const terminalTouched = useRef(false);
  if (terminalOpen) terminalTouched.current = true;
  const { dark, toggle } = useTheme();

  useHotkeys([
    { key: "k", mod: true, allowInInput: true, run: () => useSessionStore.getState().togglePalette() },
    { key: "n", mod: true, run: () => useSessionStore.getState().endSession() },
    { key: "\\", mod: true, allowInInput: true, run: () => useSessionStore.getState().toggleSidebar() },
    {
      key: "j",
      mod: true,
      run: () => {
        if (useSessionStore.getState().session) useSessionStore.getState().toggleTerminal();
      },
    },
    { key: ".", mod: true, allowInInput: true, run: () => void useSessionStore.getState().cancelActive() },
    { key: ",", mod: true, allowInInput: true, run: () => useSessionStore.getState().setSettingsOpen(true) },
    { key: "Tab", shift: true, allowInInput: true, run: () => useSessionStore.getState().cyclePermissionMode() },
  ]);

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-bg text-ink">
      <MobileRemoteAgent />
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar dark={dark} onToggleTheme={toggle} />
        <OfflineBanner />
        <CreditBanner />
        {/* The opening screen takes priority over a still-mounted previous
            session: switching conversations (especially local ↔ remote, which
            reconnects SSH) keeps the old `session` set until the new one is
            live, and leaving the old chat interactive during that window
            looked frozen — and invited input into a session being replaced.
            Peeks are excluded: the live conversation stays up while they load. */}
        {openingScreen ? (
          <OpeningScreen />
        ) : session ? (
          <>
            <Conversation />
            <Composer />
            {terminalTouched.current && (
              <Suspense
                fallback={
                  terminalOpen ? (
                    <div className="flex h-10 shrink-0 items-center gap-2 border-t border-border px-4 text-xs text-ink-muted">
                      <span className="size-3 animate-[spin_1s_linear_infinite] rounded-full border border-ink-faint border-t-transparent" />
                      Loading terminal…
                    </div>
                  ) : null
                }
              >
                <TerminalPanel />
              </Suspense>
            )}
          </>
        ) : (
          <>
            <StartCard />
            <Composer />
          </>
        )}
      </div>
      {mcpOpen && (
        <Suspense fallback={null}>
          <McpSettings />
        </Suspense>
      )}
      {sshOpen && (
        <Suspense fallback={null}>
          <SshSettings />
        </Suspense>
      )}
      {settingsOpen && (
        <Suspense fallback={null}>
          <Settings dark={dark} onToggleTheme={toggle} />
        </Suspense>
      )}
      <CommandPalette dark={dark} onToggleTheme={toggle} />
    </div>
  );
}
