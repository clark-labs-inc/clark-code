import { lazy, Suspense, useEffect, useState } from "react";
import { useSessionStore } from "./store/sessionStore";
import { useWindowFileDropGuard } from "./lib/attachmentSources";
import { useHotkeys } from "./lib/hotkeys";
import { useTextSize } from "./lib/useTextSize";
import { useTheme } from "./lib/useTheme";
import { useAppVersion } from "./lib/appInfo";
import { SignInScreen } from "./surfaces/SignInScreen";
import { UpdateStatus } from "./surfaces/UpdateStatus";
import { NoticeToast, TextSizeToast, WarningToast } from "./surfaces/Toast";
import { productModule } from "./product/productModule";

const AuthenticatedWorkspace = lazy(() => import("./AuthenticatedWorkspace"));

function WorkspaceLoadingScreen() {
  const initial = productModule().branding.shortName.slice(0, 1).toLowerCase();
  return (
    <div className="grid h-screen w-screen place-items-center bg-bg text-ink">
      <div className="breathe grid size-12 place-items-center rounded-xl border border-border-subtle bg-bg-elevated text-lg font-semibold">
        {initial}
      </div>
    </div>
  );
}

function AppVersionBadge() {
  const version = useAppVersion();
  const product = productModule();
  if (!version) return null;

  return (
    <span
      aria-label={`${product.branding.name} version ${version}`}
      className="pointer-events-none fixed bottom-2 right-3 z-10 font-mono text-xs tabular-nums text-ink-faint"
    >
      v{version}
    </span>
  );
}

export default function App() {
  const product = productModule();
  const init = useSessionStore((s) => s.init);
  const auth = useSessionStore((s) => s.auth);
  const addFiles = useSessionStore((s) => s.addFiles);
  const { textSize, setTextSize, increaseTextSize, decreaseTextSize, resetTextSize } = useTextSize();
  const { dark, toggle, colorblind, toggleColorblind } = useTheme();
  const [textSizeToastSignal, setTextSizeToastSignal] = useState(0);

  const runTextSizeShortcut = (action: () => void) => {
    action();
    setTextSizeToastSignal((signal) => signal + 1);
  };

  useHotkeys([
    // KeyboardEvent.key varies between "+" and "=" across layouts/keypads.
    { key: "+", mod: true, shift: true, allowInInput: true, run: () => runTextSizeShortcut(increaseTextSize) },
    { key: "+", mod: true, allowInInput: true, run: () => runTextSizeShortcut(increaseTextSize) },
    { key: "=", mod: true, shift: true, allowInInput: true, run: () => runTextSizeShortcut(increaseTextSize) },
    { key: "=", mod: true, allowInInput: true, run: () => runTextSizeShortcut(increaseTextSize) },
    { key: "-", mod: true, allowInInput: true, run: () => runTextSizeShortcut(decreaseTextSize) },
    { key: "0", mod: true, allowInInput: true, run: () => runTextSizeShortcut(resetTextSize) },
  ]);

  useEffect(() => {
    void init();
  }, [init]);

  // File drops land on the composer when it's open; anywhere else in the
  // window they still attach (and never navigate the webview away). On the
  // sign-in screen they're only swallowed, not attached.
  useWindowFileDropGuard(auth || !product.authRequired ? (files) => void addFiles(files) : undefined);

  if (!auth && product.authRequired)
    return (
      <>
        <SignInScreen />
        <UpdateStatus />
        <NoticeToast />
        <TextSizeToast textSize={textSize} signal={textSizeToastSignal} />
        <AppVersionBadge />
      </>
    );

  return (
    <>
      <Suspense fallback={<WorkspaceLoadingScreen />}>
        <AuthenticatedWorkspace
          textSize={textSize}
          onTextSizeChange={setTextSize}
          dark={dark}
          onToggleTheme={toggle}
          colorblind={colorblind}
          onToggleColorblind={toggleColorblind}
        />
      </Suspense>
      <UpdateStatus />
      <NoticeToast />
      <WarningToast />
      <TextSizeToast textSize={textSize} signal={textSizeToastSignal} />
      <AppVersionBadge />
    </>
  );
}
