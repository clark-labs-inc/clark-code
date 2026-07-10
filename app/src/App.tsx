import { lazy, Suspense, useEffect } from "react";
import { useSessionStore } from "./store/sessionStore";
import { SignInScreen } from "./surfaces/SignInScreen";
import { UpdateStatus } from "./surfaces/UpdateStatus";
import { NoticeToast } from "./surfaces/Toast";

const AuthenticatedWorkspace = lazy(() => import("./AuthenticatedWorkspace"));

function WorkspaceLoadingScreen() {
  return (
    <div className="grid h-screen w-screen place-items-center bg-bg text-ink">
      <div className="breathe grid size-12 place-items-center rounded-xl border border-border-subtle bg-bg-elevated text-lg font-semibold">
        c
      </div>
    </div>
  );
}

export default function App() {
  const init = useSessionStore((s) => s.init);
  const auth = useSessionStore((s) => s.auth);

  useEffect(() => {
    void init();
  }, [init]);

  if (!auth)
    return (
      <>
        <SignInScreen />
        <UpdateStatus />
        <NoticeToast />
      </>
    );

  return (
    <>
      <Suspense fallback={<WorkspaceLoadingScreen />}>
        <AuthenticatedWorkspace />
      </Suspense>
      <UpdateStatus />
      <NoticeToast />
    </>
  );
}
