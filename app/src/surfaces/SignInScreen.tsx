import { useState } from "react";
import { motion } from "motion/react";
import { Loader2 } from "lucide-react";
import { ClarkMark } from "./ClarkMark";
import { useSessionStore } from "../store/sessionStore";
import { isGoogleConfigured } from "../lib/auth";

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
    <div className="flex h-screen w-screen items-center justify-center bg-bg px-6">
      <motion.div
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.3 }}
        className="w-full max-w-sm text-center"
      >
        <div className="mb-6 flex justify-center">
          <ClarkMark size={64} className="rounded-2xl" />
        </div>
        <h1 className="text-2xl font-semibold tracking-tight text-ink">Clark Code</h1>
        <p className="mt-2 text-sm text-ink-muted">
          A coding agent on your machine — your files, your shell, your model,
          with Clark for research.
        </p>

        <div className="mt-8">
          <button
            onClick={() => void go()}
            disabled={busy}
            className="flex w-full items-center justify-center gap-3 rounded-xl bg-ink px-4 py-3 text-sm font-semibold text-bg transition hover:bg-accent-hover disabled:opacity-60"
          >
            {busy ? (
              <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" />
            ) : (
              <GoogleG className="size-4" />
            )}
            Continue with Google
          </button>
        </div>

        {!isGoogleConfigured() && (
          <p className="mt-3 text-xs text-ink-faint">
            Google sign-in activates once <span className="font-mono">VITE_CLARK_AUTH_ORIGIN</span>
            {" "}(and a desktop client id) is configured.
          </p>
        )}
        {error && <p className="mt-3 text-xs text-danger">{error}</p>}

        <p className="mt-8 text-xs text-ink-faint">Private beta · Clark Code</p>
      </motion.div>
    </div>
  );
}
