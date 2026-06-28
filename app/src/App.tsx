import { useEffect } from "react";
import { useSessionStore } from "./store/sessionStore";
import { useTheme } from "./lib/useTheme";
import { TopBar } from "./surfaces/TopBar";
import { Sidebar } from "./surfaces/Sidebar";
import { StartCard } from "./surfaces/StartCard";
import { Conversation } from "./surfaces/Conversation";
import { Composer } from "./surfaces/Composer";
import { TerminalPanel } from "./surfaces/TerminalPanel";
import { CreditBanner } from "./surfaces/CreditBanner";
import { McpSettings } from "./surfaces/McpSettings";
import { SignInScreen } from "./surfaces/SignInScreen";

export default function App() {
  const init = useSessionStore((s) => s.init);
  const auth = useSessionStore((s) => s.auth);
  const session = useSessionStore((s) => s.session);
  const { dark, toggle } = useTheme();

  useEffect(() => {
    void init();
  }, [init]);

  if (!auth) return <SignInScreen />;

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-bg text-ink">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar dark={dark} onToggleTheme={toggle} />
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
    </div>
  );
}
