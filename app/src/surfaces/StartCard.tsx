import { motion } from "motion/react";
import { ArrowRight } from "lucide-react";
import { ClarkMark } from "./ClarkMark";
import { useSessionStore } from "../store/sessionStore";

const SAMPLES = [
  "In one sentence, what is the Rust programming language?",
  "Create /home/user/workspace/notes.txt with three lines, then read it back and replace one word.",
  "Build a one-page website about cats and publish it. Give me the URL.",
];

export function StartCard() {
  const start = useSessionStore((s) => s.startSession);
  const connecting = useSessionStore((s) => s.connecting);
  const error = useSessionStore((s) => s.error);

  const startWith = async (q?: string) => {
    await start();
    if (q) await useSessionStore.getState().send(q);
  };

  return (
    <div className="flex flex-1 items-center justify-center overflow-y-auto p-6">
      <motion.div
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.25 }}
        className="w-full max-w-lg"
      >
        <div className="mb-5 flex flex-col items-center text-center">
          <ClarkMark size={44} className="mb-3 rounded-xl" />
          <h1 className="text-lg font-semibold text-ink">Start a session</h1>
          <p className="mt-1 text-sm text-ink-muted">
            One window. Watch every step — files, web, and computer work — as it happens.
          </p>
        </div>

        <button
          onClick={() => void startWith()}
          disabled={connecting}
          className="mb-4 flex w-full items-center justify-center gap-2 rounded-xl bg-accent px-3 py-3 text-sm font-semibold text-on-accent transition hover:bg-accent-hover disabled:opacity-50"
        >
          {connecting ? "Connecting…" : "New session"}
          {!connecting && <ArrowRight className="size-4" />}
        </button>

        {error && <p className="mb-3 text-center text-xs text-danger">{error}</p>}

        <div className="text-xs text-ink-faint">
          <p className="mb-1.5 font-medium uppercase tracking-wider">Try</p>
          <div className="space-y-1.5">
            {SAMPLES.map((s) => (
              <button
                key={s}
                onClick={() => void startWith(s)}
                disabled={connecting}
                className="block w-full truncate rounded-lg border border-border-subtle bg-bg-elevated/60 px-3 py-2.5 text-left text-ink-secondary transition hover:bg-bg-hover disabled:opacity-50"
              >
                {s}
              </button>
            ))}
          </div>
        </div>
      </motion.div>
    </div>
  );
}
