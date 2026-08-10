import { useState } from "react";
import { productName } from "../product/productModule";
import { useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { Loader2, Download, RotateCw, Check, AlertCircle } from "lucide-react";
import { ProductMark } from "./ProductMark";
import { useSessionStore } from "../store/sessionStore";
import {
  DUR,
  RISE,
  accessibleMotion,
  staggeredTransition,
} from "../lib/motion";

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Update affordance shown on the sign-in screen itself, so a user stuck on a
 *  broken build (e.g. one where sign-in was misconfigured) can pull the fix
 *  without first getting past sign-in. Reuses the store's existing
 *  check/stage/apply state machine — the same one that auto-checks on launch. */
function UpdateControl() {
  const update = useSessionStore((s) => s.update);
  const progress = useSessionStore((s) => s.updateProgress);
  const checking = useSessionStore((s) => s.updateChecking);
  const applying = useSessionStore((s) => s.updateApplying);
  const waiting = useSessionStore((s) => s.updateWaiting);
  const checkForUpdate = useSessionStore((s) => s.checkForUpdate);
  const applyUpdate = useSessionStore((s) => s.applyUpdate);
  const [upToDate, setUpToDate] = useState(false);
  const [checkError, setCheckError] = useState<string | null>(null);

  // A verified update is downloaded and staged — offer to restart into it.
  if (update) {
    return (
      <button
        onClick={() => void applyUpdate()}
        disabled={applying || waiting}
        className="mt-6 flex w-full items-center justify-center gap-2 rounded-xl border border-accent px-4 py-2.5 text-sm font-semibold text-accent transition hover:bg-accent hover:text-on-accent disabled:opacity-60"
      >
        {applying || waiting ? (
          <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" />
        ) : (
          <Download className="size-4" />
        )}
        {waiting ? "Finishing active work before update…" : `Restart to update to ${update.version}`}
      </button>
    );
  }

  // Downloading + verifying in the background.
  if (progress) {
    const pct =
      progress.total && progress.total > 0
        ? Math.round((progress.downloaded / progress.total) * 100)
        : null;
    return (
      <p className="mt-6 flex items-center justify-center gap-2 text-xs text-ink-muted">
        <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" />
        Downloading update{pct != null ? ` — ${pct}%` : "…"}
      </p>
    );
  }

  // Nothing staged: outside the desktop app there's nothing to update.
  if (!inTauri()) return null;

  const check = async () => {
    setUpToDate(false);
    setCheckError(null);
    const result = await checkForUpdate();
    if (result.status === "up-to-date") setUpToDate(true);
    if (result.status === "error") setCheckError(result.message);
  };

  return (
    <div className="mt-6 flex flex-col items-center gap-1">
      <button
        data-qa="check-for-updates"
        onClick={() => void check()}
        disabled={checking}
        className="flex items-center justify-center gap-1.5 text-xs font-medium text-ink-muted transition hover:text-ink disabled:opacity-60"
      >
        {checking ? (
          <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" />
        ) : (
          <RotateCw className="size-3.5" />
        )}
        {checking ? "Checking for updates…" : "Check for updates"}
      </button>
      {upToDate && (
        <span className="flex items-center gap-1 text-xs text-ink-faint">
          <Check className="size-3" /> You’re on the latest version.
        </span>
      )}
      {checkError && (
        <span className="flex items-center gap-1 text-xs text-danger" title={checkError}>
          <AlertCircle className="size-3" /> Couldn’t check for updates. Try again.
        </span>
      )}
    </div>
  );
}

function GoogleG({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} aria-hidden>
      <path fill="#4285F4" d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 0 1-2.2 3.32v2.77h3.57c2.08-1.92 3.27-4.74 3.27-8.1z" />
      <path fill="#34A853" d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84A11 11 0 0 0 12 23z" />
      <path fill="#FBBC05" d="M5.84 14.1a6.6 6.6 0 0 1 0-4.2V7.06H2.18a11 11 0 0 0 0 9.88l3.66-2.84z" />
      <path fill="#EA4335" d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1A11 11 0 0 0 2.18 7.06l3.66 2.84C6.71 7.31 9.14 5.38 12 5.38z" />
    </svg>
  );
}

export function SignInScreen() {
  const signIn = useSessionStore((s) => s.signIn);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const reduce = useReducedMotion() ?? false;

  const go = async () => {
    setBusy(true);
    setError(null);
    try {
      await signIn("google");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  return (
    <div data-qa="sign-in-screen" className="flex h-screen w-screen items-center justify-center bg-bg px-6">
      <m.div
        {...accessibleMotion(RISE, reduce)}
        transition={staggeredTransition(reduce, 0, 0.04, { duration: DUR.slow })}
        className="w-full max-w-sm text-center"
      >
        <div className="mb-6 flex justify-center">
          <ProductMark size={64} className="rounded-2xl" />
        </div>
        <h1 className="text-2xl font-semibold tracking-tight text-ink">{productName()}</h1>
        <p className="mt-2 text-sm text-ink-muted">
          A coding agent on your machine — your files, your shell, your model,
          with the agent for research.
        </p>

        <div className="mt-8">
          <button
            data-qa="sign-in-google"
            onClick={() => void go()}
            disabled={busy}
            className="flex min-h-11 w-full items-center justify-center gap-3 rounded-xl bg-accent px-4 py-3 text-sm font-semibold text-on-accent shadow-soft transition duration-200 ease-agent hover:-translate-y-0.5 hover:bg-accent-hover active:translate-y-0 disabled:translate-y-0 disabled:opacity-60"
          >
            {busy ? (
              <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" />
            ) : (
              <GoogleG className="size-4" />
            )}
            Continue with Google
          </button>
        </div>

        {error && <p className="mt-3 text-xs text-danger">{error}</p>}

        <UpdateControl />

        <p className="mt-8 text-xs text-ink-faint">Open source · {productName()}</p>
      </m.div>
    </div>
  );
}
