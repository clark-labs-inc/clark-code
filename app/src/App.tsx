import { useEffect } from "react";
import { useSessionStore } from "./store/sessionStore";
import { useTheme } from "./lib/useTheme";
import { TopBar } from "./surfaces/TopBar";
import { Sidebar } from "./surfaces/Sidebar";
import { StartCard } from "./surfaces/StartCard";
import { Conversation } from "./surfaces/Conversation";
import { Composer } from "./surfaces/Composer";
import { SignInScreen } from "./surfaces/SignInScreen";

export default function App() {
  const init = useSessionStore((s) => s.init);
  const auth = useSessionStore((s) => s.auth);
  const session = useSessionStore((s) => s.session);
  const { dark, toggle } = useTheme();

  useEffect(() => {
    void init();
  }, [init]);

  // Dev/demo affordance: `?q=...` (or `?demo`) signs in with the demo account,
  // starts a Clark session, and sends a prompt — so the UI can be exercised,
  // screenshotted, and screen-recorded without manual clicks.
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    if (!params.has("demo") && !params.has("q")) return;
    const query =
      params.get("q") ?? "Look at src/main.rs and tell me what it does.";
    let cancelled = false;
    const store = useSessionStore.getState;
    void (async () => {
      while (!cancelled && store().providers.length === 0) {
        await new Promise((r) => setTimeout(r, 30));
      }
      if (cancelled) return;
      if (!store().auth) await store().signIn("demo");
      await store().startSession();
      if (cancelled) return;
      await store().send(query);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  if (!auth) return <SignInScreen />;

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-bg text-ink">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar dark={dark} onToggleTheme={toggle} />
        {session ? (
          <>
            <Conversation />
            <Composer />
          </>
        ) : (
          <StartCard />
        )}
      </div>
    </div>
  );
}
