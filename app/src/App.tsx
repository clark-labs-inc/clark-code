import { lazy, Suspense, useEffect, useRef } from "react";
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
import { SignInScreen } from "./surfaces/SignInScreen";
import { MobileRemoteAgent } from "./surfaces/MobileRemoteAgent";
import { UpdateStatus } from "./surfaces/UpdateStatus";
import { NoticeToast } from "./surfaces/Toast";

// Heavy, on-demand surfaces stay OUT of the startup bundle (xterm alone is
// ~350KB): the terminal loads on first open (then stays mounted so the PTY
// survives hide/show), and the settings modals load when opened.
const TerminalPanel = lazy(() =>
  import("./surfaces/TerminalPanel").then((m) => ({ default: m.TerminalPanel })),
);
const McpSettings = lazy(() =>
  import("./surfaces/McpSettings").then((m) => ({ default: m.McpSettings })),
);
const SshSettings = lazy(() =>
  import("./surfaces/SshSettings").then((m) => ({ default: m.SshSettings })),
);
const Settings = lazy(() =>
  import("./surfaces/Settings").then((m) => ({ default: m.Settings })),
);

export default function App() {
  const init = useSessionStore((s) => s.init);
  const auth = useSessionStore((s) => s.auth);
  const session = useSessionStore((s) => s.session);
  const opening = useSessionStore((s) => s.opening);
  const terminalOpen = useSessionStore((s) => s.terminalOpen);
  const mcpOpen = useSessionStore((s) => s.mcpOpen);
  const sshOpen = useSessionStore((s) => s.sshOpen);
  const settingsOpen = useSessionStore((s) => s.settingsOpen);
  // Latch: once the terminal has been opened, keep it mounted so its PTY
  // sessions survive hide/show (the panel gates its own visibility).
  const terminalTouched = useRef(false);
  if (terminalOpen) terminalTouched.current = true;
  const { dark, toggle } = useTheme();

  useEffect(() => {
    void init();
  }, [init]);

  // Global keyboard map — the desktop power-user surface. Actions read live
  // store state at call time so the bindings never go stale.
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

  if (!auth)
    return (
      <>
        <SignInScreen />
        <UpdateStatus />
        <NoticeToast />
      </>
    );

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-bg text-ink">
      <MobileRemoteAgent />
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar dark={dark} onToggleTheme={toggle} />
        <OfflineBanner />
        <CreditBanner />
        {session ? (
          <>
            <Conversation />
            <Composer />
            {terminalTouched.current && (
              <Suspense fallback={null}>
                <TerminalPanel />
              </Suspense>
            )}
          </>
        ) : opening ? (
          <OpeningScreen />
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
      <UpdateStatus />
      <NoticeToast />
    </div>
  );
}
