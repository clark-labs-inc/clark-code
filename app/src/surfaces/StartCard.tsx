import { motion } from "motion/react";
import { ArrowRight, FolderOpen, Folder } from "lucide-react";
import { ClarkMark } from "./ClarkMark";
import { useSessionStore } from "../store/sessionStore";
import { localSettingsReady, projectName, type LocalAgentSettings } from "../lib/localAgent";
import { inTauri } from "../lib/pickFolder";

const SAMPLES = [
  "In one sentence, what is the Rust programming language?",
  "Create /home/user/workspace/notes.txt with three lines, then read it back and replace one word.",
  "Build a one-page website about cats and publish it. Give me the URL.",
];

const LOCAL_SAMPLES = [
  "Summarize what this project does from its README and top-level files.",
  "Find every TODO in the codebase and list them by file.",
  "Add a unit test for the function in the file I'm about to mention.",
];

export function StartCard() {
  const start = useSessionStore((s) => s.startSession);
  const connecting = useSessionStore((s) => s.connecting);
  const error = useSessionStore((s) => s.error);
  const providers = useSessionStore((s) => s.providers);
  const activeProvider = useSessionStore((s) => s.activeProvider);
  const selectProvider = useSessionStore((s) => s.selectProvider);
  const local = useSessionStore((s) => s.localSettings);

  const isLocal = activeProvider === "local";
  const blocked = isLocal ? localSettingsReady(local) : null;

  const startWith = async (q?: string) => {
    if (blocked) return;
    await start();
    if (q) await useSessionStore.getState().send(q);
  };

  const samples = isLocal ? LOCAL_SAMPLES : SAMPLES;

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
            {isLocal
              ? "Code on your machine with a local agent loop — your files, your shell, your model."
              : "One window. Watch every step — files, web, and computer work — as it happens."}
          </p>
        </div>

        {providers.length > 1 && (
          <div className="mb-4 flex gap-1 rounded-xl border border-border-subtle bg-bg-elevated/60 p-1">
            {providers.map((p) => (
              <button
                key={p.id}
                onClick={() => selectProvider(p.id)}
                className={`flex-1 rounded-lg px-3 py-1.5 text-sm font-medium transition ${
                  p.id === activeProvider
                    ? "bg-accent text-on-accent"
                    : "text-ink-secondary hover:bg-bg-hover"
                }`}
              >
                {p.label}
              </button>
            ))}
          </div>
        )}

        {isLocal && <LocalSettingsForm settings={local} />}

        <button
          onClick={() => void startWith()}
          disabled={connecting || !!blocked}
          className="mb-2 flex w-full items-center justify-center gap-2 rounded-xl bg-accent px-3 py-3 text-sm font-semibold text-on-accent transition hover:bg-accent-hover disabled:opacity-50"
        >
          {connecting ? "Connecting…" : "New session"}
          {!connecting && <ArrowRight className="size-4" />}
        </button>

        {blocked && <p className="mb-3 text-center text-xs text-ink-faint">{blocked}</p>}
        {error && <p className="mb-3 text-center text-xs text-danger">{error}</p>}

        <div className="mt-2 text-xs text-ink-faint">
          <p className="mb-1.5 font-medium uppercase tracking-wider">Try</p>
          <div className="space-y-1.5">
            {samples.map((s) => (
              <button
                key={s}
                onClick={() => void startWith(s)}
                disabled={connecting || !!blocked}
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

function LocalSettingsForm({ settings }: { settings: LocalAgentSettings }) {
  const extractMemory = useSessionStore((s) => s.extractMemory);
  const extracting = useSessionStore((s) => s.extractingMemory);
  const memoryStatus = useSessionStore((s) => s.memoryStatus);
  const canExtract = !!settings.cwd.trim() && !!settings.apiKey.trim();

  return (
    <div className="mb-4 space-y-3 rounded-xl border border-border-subtle bg-bg-elevated/40 p-3">
      <ProjectFolderField cwd={settings.cwd} />

      <p className="text-xs text-ink-muted">
        Clark Code is connected through your account — no API key needed. Coding runs
        on this machine; the model and research run on Clark.
      </p>

      <div className="border-t border-border-subtle pt-3">
        <button
          type="button"
          onClick={() => void extractMemory()}
          disabled={!canExtract || extracting}
          className="w-full rounded-lg border border-border bg-bg px-3 py-2 text-sm font-medium text-ink-secondary transition hover:bg-bg-hover disabled:opacity-50"
        >
          {extracting ? "Extracting project memory…" : "Extract project memory with Clark"}
        </button>
        <p className="mt-1 text-xs text-ink-muted">
          Clark analyzes the repo and writes <code>.clark/memory/MEMORY.md</code>, which the agent
          reads every session.
        </p>
        {memoryStatus && <p className="mt-1 text-[11px] text-ink-secondary">{memoryStatus}</p>}
      </div>
    </div>
  );
}

function ProjectFolderField({ cwd }: { cwd: string }) {
  const pick = useSessionStore((s) => s.pickProjectFolder);
  const setProject = useSessionStore((s) => s.setProjectFolder);
  const setLocal = useSessionStore((s) => s.setLocalSettings);
  const recents = useSessionStore((s) => s.recentProjects);
  const tauri = inTauri();

  return (
    <div>
      <label className="mb-1 block text-xs font-medium text-ink-secondary">Project folder</label>
      <div className="flex items-stretch gap-2">
        {tauri && (
          <button
            type="button"
            onClick={() => void pick()}
            className="flex shrink-0 items-center gap-1.5 rounded-lg bg-accent px-3 py-2 text-sm font-medium text-on-accent transition hover:bg-accent-hover"
          >
            <FolderOpen className="size-4" /> Choose…
          </button>
        )}
        <input
          type="text"
          value={cwd}
          onChange={(e) => setLocal({ cwd: e.target.value })}
          placeholder={tauri ? "…or paste an absolute path" : "/Users/you/code/my-project"}
          spellCheck={false}
          className={`${inputCls} flex-1`}
        />
      </div>
      {cwd.trim() && (
        <p className="mt-1 flex items-center gap-1.5 truncate text-xs text-ink-muted">
          <Folder className="size-3 shrink-0" />
          <span className="font-medium text-ink-secondary">{projectName(cwd)}</span>
          <span className="truncate">{cwd}</span>
        </p>
      )}
      {recents.length > 0 && (
        <div className="mt-2">
          <p className="mb-1 text-[11px] uppercase tracking-wider text-ink-faint">Recent</p>
          <div className="flex flex-wrap gap-1.5">
            {recents.map((p) => (
              <button
                key={p}
                type="button"
                title={p}
                onClick={() => setProject(p)}
                className={`flex max-w-[12rem] items-center gap-1 rounded-md border px-2 py-1 text-xs transition ${
                  p === cwd
                    ? "border-accent bg-accent/10 text-ink"
                    : "border-border-subtle bg-bg-elevated/60 text-ink-secondary hover:bg-bg-hover"
                }`}
              >
                <Folder className="size-3 shrink-0" />
                <span className="truncate">{projectName(p)}</span>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

const inputCls =
  "w-full rounded-lg border border-border bg-bg px-2.5 py-1.5 text-sm text-ink outline-none transition focus:border-accent placeholder:text-ink-muted";
