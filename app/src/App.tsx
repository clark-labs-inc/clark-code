import { lazy, Suspense, useEffect } from "react";
import { useSessionStore } from "./store/sessionStore";
import { useWindowFileDropGuard } from "./lib/attachmentSources";
import { useHotkeys } from "./lib/hotkeys";
import { useTextSize } from "./lib/useTextSize";
import { SignInScreen } from "./surfaces/SignInScreen";
import { UpdateStatus } from "./surfaces/UpdateStatus";
import { NoticeToast, WarningToast } from "./surfaces/Toast";

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
  const addFiles = useSessionStore((s) => s.addFiles);
  const { textSize, setTextSize, increaseTextSize, decreaseTextSize, resetTextSize } = useTextSize();

  useHotkeys([
    // KeyboardEvent.key varies between "+" and "=" across layouts/keypads.
    { key: "+", mod: true, shift: true, allowInInput: true, run: increaseTextSize },
    { key: "+", mod: true, allowInInput: true, run: increaseTextSize },
    { key: "=", mod: true, shift: true, allowInInput: true, run: increaseTextSize },
    { key: "=", mod: true, allowInInput: true, run: increaseTextSize },
    { key: "-", mod: true, allowInInput: true, run: decreaseTextSize },
    { key: "0", mod: true, allowInInput: true, run: resetTextSize },
  ]);

  useEffect(() => {
    void init();
  }, [init]);

  // File drops land on the composer when it's open; anywhere else in the
  // window they still attach (and never navigate the webview away). On the
  // sign-in screen they're only swallowed, not attached.
  useWindowFileDropGuard(auth ? (files) => void addFiles(files) : undefined);

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
        <AuthenticatedWorkspace textSize={textSize} onTextSizeChange={setTextSize} />
      </Suspense>
      <UpdateStatus />
      <NoticeToast />
      <WarningToast />
    </>
  );
}
