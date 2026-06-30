import { useEffect, useRef, type ReactNode } from "react";
import { AnimatePresence, motion } from "motion/react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { NotebookText, X, RefreshCw, Sparkles, FolderGit2, FileText } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
import { cn } from "../lib/cn";
import type { MemoryFactView } from "../core-bridge/types";

// Compact markdown styling for the small memory cards.
const MD =
  "text-[12.5px] leading-relaxed text-ink-secondary " +
  "[&_p]:my-1.5 [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 " +
  "[&_ul]:my-1.5 [&_ul]:list-disc [&_ul]:pl-4 [&_ul]:marker:text-ink-faint [&_li]:my-0.5 " +
  "[&_h1]:mb-1 [&_h1]:mt-2 [&_h1]:text-sm [&_h1]:font-semibold [&_h2]:mb-1 [&_h2]:mt-2 [&_h2]:text-[13px] [&_h2]:font-semibold [&_h3]:mb-0.5 [&_h3]:mt-1.5 [&_h3]:font-semibold " +
  "[&_a]:text-ink [&_a]:underline [&_a]:decoration-ink-faint [&_strong]:font-semibold [&_strong]:text-ink " +
  "[&_code]:rounded [&_code]:border [&_code]:border-border-subtle [&_code]:bg-chip [&_code]:px-1 [&_code]:py-px [&_code]:font-mono [&_code]:text-[0.85em]";

const KIND_TONE: Record<string, string> = {
  user: "border-info/40 text-info",
  feedback: "border-warning/40 text-warning",
  project: "border-success/40 text-success",
  reference: "border-border text-ink-muted",
};

/** Top-bar control: a button that opens the per-folder memory viewer. */
export function MemoryButton() {
  const open = useSessionStore((s) => s.memoryViewerOpen);
  const toggle = useSessionStore((s) => s.toggleMemoryViewer);
  const setOpen = useSessionStore((s) => s.setMemoryViewerOpen);
  const wrapRef = useRef<HTMLDivElement>(null);

  // Close on outside click / Escape.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, setOpen]);

  return (
    <div ref={wrapRef} className="relative">
      <button
        onClick={toggle}
        aria-label={open ? "Hide project memory" : "Show project memory"}
        title="Project memory (what the agent remembers about this folder)"
        className={cn(
          "grid size-8 place-items-center rounded-lg transition",
          open
            ? "bg-bg-hover text-ink"
            : "text-ink-muted hover:bg-bg-hover hover:text-ink-secondary",
        )}
      >
        <NotebookText className="size-4" />
      </button>
      <AnimatePresence>{open && <MemoryPopover key="memory-popover" />}</AnimatePresence>
    </div>
  );
}

function MemoryPopover() {
  const setOpen = useSessionStore((s) => s.setMemoryViewerOpen);
  const loading = useSessionStore((s) => s.loadingMemory);
  const overview = useSessionStore((s) => s.memoryOverview);
  const reload = useSessionStore((s) => s.loadMemory);
  const cwd = useSessionStore((s) => s.localSettings.cwd);

  const isEmpty = !loading && overview && !overview.exists && overview.facts.length === 0;

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.1 }}
      className="popover-surface absolute right-0 top-10 z-50 flex max-h-[70vh] w-[26rem] flex-col overflow-hidden rounded-xl border border-border bg-bg-elevated shadow-xl"
    >
      <header className="flex items-center gap-2 border-b border-border-subtle px-3 py-2.5">
        <NotebookText className="size-4 shrink-0 text-ink-muted" />
        <div className="min-w-0">
          <p className="text-sm font-medium text-ink">Project memory</p>
          <p className="flex items-center gap-1 truncate text-[11px] text-ink-faint">
            <FolderGit2 className="size-3 shrink-0" />
            <span className="truncate" title={cwd}>
              {cwd ? projectName(cwd) : "No folder"}
            </span>
          </p>
        </div>
        <button
          onClick={() => void reload()}
          disabled={loading}
          title="Reload from disk"
          aria-label="Reload memory"
          className="ml-auto grid size-7 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:opacity-50"
        >
          <RefreshCw className={cn("size-3.5", loading && "animate-spin")} />
        </button>
        <button
          onClick={() => setOpen(false)}
          aria-label="Close"
          className="grid size-7 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink"
        >
          <X className="size-3.5" />
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3">
        {loading && !overview ? (
          <p className="py-6 text-center text-xs text-ink-faint">Reading memory…</p>
        ) : !overview ? (
          <p className="py-6 text-center text-xs text-ink-faint">
            Project memory is available in the desktop app.
          </p>
        ) : isEmpty ? (
          <div className="py-4 text-center">
            <p className="text-sm font-medium text-ink-secondary">No memory yet</p>
            <p className="mx-auto mt-1 max-w-[20rem] text-xs text-ink-muted">
              The agent maintains durable notes about this folder under{" "}
              <code className="rounded bg-chip px-1 py-px font-mono text-[0.85em]">
                .clark/memory
              </code>
              . Extract one with Clark to bootstrap it.
            </p>
          </div>
        ) : (
          <div className="space-y-3">
            {overview?.index && (
              <section>
                <SectionLabel icon={<FileText className="size-3" />} label="MEMORY.md (index)" />
                <div className="rounded-lg border border-border-subtle bg-bg-sunken/60 px-3 py-2">
                  <div className={MD}>
                    <Markdown remarkPlugins={[remarkGfm]}>{overview.index}</Markdown>
                  </div>
                </div>
              </section>
            )}
            {overview && overview.facts.length > 0 && (
              <section>
                <SectionLabel
                  icon={<NotebookText className="size-3" />}
                  label={`${overview.facts.length} memory ${overview.facts.length === 1 ? "note" : "notes"}`}
                />
                <div className="space-y-2">
                  {overview.facts.map((f) => (
                    <FactCard key={f.file} fact={f} />
                  ))}
                </div>
              </section>
            )}
          </div>
        )}
      </div>

      <ExtractFooter />
    </motion.div>
  );
}

function SectionLabel({ icon, label }: { icon: ReactNode; label: string }) {
  return (
    <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wider text-ink-faint">
      {icon}
      {label}
    </div>
  );
}

function FactCard({ fact }: { fact: MemoryFactView }) {
  const tone = (fact.kind && KIND_TONE[fact.kind]) || "border-border text-ink-muted";
  return (
    <details className="group rounded-lg border border-border-subtle bg-bg-sunken/40 px-3 py-2">
      <summary className="flex cursor-pointer list-none items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-xs font-medium text-ink-secondary">
          {fact.name || fact.description || fact.file}
        </span>
        {fact.kind && (
          <span
            className={cn(
              "shrink-0 rounded-full border px-1.5 py-px text-[10px] font-medium",
              tone,
            )}
          >
            {fact.kind}
          </span>
        )}
      </summary>
      {fact.description && fact.name && (
        <p className="mt-1 text-[11px] text-ink-muted">{fact.description}</p>
      )}
      {fact.body && (
        <div className={cn(MD, "mt-1.5 border-t border-border-subtle pt-1.5")}>
          <Markdown remarkPlugins={[remarkGfm]}>{fact.body}</Markdown>
        </div>
      )}
      <p className="mt-1.5 font-mono text-[10px] text-ink-faint">{fact.file}</p>
    </details>
  );
}

/** Footer with the "update / extract" action — re-runs Clark extraction. */
function ExtractFooter() {
  const extract = useSessionStore((s) => s.extractMemory);
  const extracting = useSessionStore((s) => s.extractingMemory);
  const status = useSessionStore((s) => s.memoryStatus);
  const hasMemory = useSessionStore((s) => !!s.memoryOverview?.exists);
  const canExtract = useSessionStore(
    (s) => !!s.localSettings.cwd.trim() && !!s.localSettings.apiKey.trim(),
  );

  return (
    <footer className="border-t border-border-subtle px-3 py-2.5">
      <button
        type="button"
        onClick={() => void extract()}
        disabled={!canExtract || extracting}
        title={
          canExtract
            ? "Clark re-reads the repo and rewrites MEMORY.md"
            : "Add your Clark API key to extract memory"
        }
        className="flex w-full items-center justify-center gap-2 rounded-lg border border-border bg-bg px-3 py-2 text-sm font-medium text-ink-secondary transition hover:bg-bg-hover disabled:opacity-50"
      >
        {extracting ? (
          <>
            <RefreshCw className="size-3.5 animate-spin" />
            Extracting…
          </>
        ) : (
          <>
            <Sparkles className="size-3.5" />
            {hasMemory ? "Update memory with Clark" : "Extract memory with Clark"}
          </>
        )}
      </button>
      {status && <p className="mt-1.5 text-[11px] text-ink-muted">{status}</p>}
    </footer>
  );
}
