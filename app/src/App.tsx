import { useEffect } from "react";
import { useSessionStore } from "./store/sessionStore";
import { useTheme } from "./lib/useTheme";
import { useHotkeys } from "./lib/hotkeys";
import { TopBar } from "./surfaces/TopBar";
import { Sidebar } from "./surfaces/Sidebar";
import { StartCard } from "./surfaces/StartCard";
import { Conversation } from "./surfaces/Conversation";
import { Composer } from "./surfaces/Composer";
import { TerminalPanel } from "./surfaces/TerminalPanel";
import { CreditBanner } from "./surfaces/CreditBanner";
import { OfflineBanner } from "./surfaces/OfflineBanner";
import { McpSettings } from "./surfaces/McpSettings";
import { SshSettings } from "./surfaces/SshSettings";
import { CommandPalette } from "./surfaces/CommandPalette";
import { SignInScreen } from "./surfaces/SignInScreen";

export default function App() {
  const init = useSessionStore((s) => s.init);
  const auth = useSessionStore((s) => s.auth);
  const session = useSessionStore((s) => s.session);
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
  ]);

  if (!auth) return <SignInScreen />;

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-bg text-ink">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar dark={dark} onToggleTheme={toggle} />
        <OfflineBanner />
        <CreditBanner />
        {session ? (
          <>
            <Conversation />
            <Composer />
            <TerminalPanel />
          </>
        ) : (
          <StartCard />
        )}
      </div>
      <McpSettings />
      <SshSettings />
      <CommandPalette dark={dark} onToggleTheme={toggle} />
    </div>
  );
}
