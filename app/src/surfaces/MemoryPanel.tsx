import { useEffect, useRef, type ReactNode } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { NotebookText, X, RefreshCw, FolderGit2, Globe, FileText } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
import { cn } from "../lib/cn";
import type { MemoryFactView, MemoryOverview } from "../core-bridge/types";

// Compact markdown styling for the small memory cards.
const MD =
  "text-sm leading-relaxed text-ink-secondary " +
  "[&_p]:my-1.5 [&_p:first-child]:mt-0 [&_p:last-child]:mb-0 " +
  "[&_ul]:my-1.5 [&_ul]:list-disc [&_ul]:pl-4 [&_ul]:marker:text-ink-faint [&_li]:my-0.5 " +
  "[&_h1]:mb-1 [&_h1]:mt-2 [&_h1]:text-sm [&_h1]:font-semibold [&_h2]:mb-1 [&_h2]:mt-2 [&_h2]:text-sm [&_h2]:font-semibold [&_h3]:mb-0.5 [&_h3]:mt-1.5 [&_h3]:font-semibold " +
  "[&_a]:text-ink [&_a]:underline [&_a]:decoration-ink-faint [&_strong]:font-semibold [&_strong]:text-ink " +
  "[&_code]:rounded [&_code]:border [&_code]:border-border-subtle [&_code]:bg-chip [&_code]:px-1 [&_code]:py-px [&_code]:font-mono [&_code]:text-[0.85em]";

const KIND_TONE: Record<string, string> = {
  user: "border-info/40 text-info",
  feedback: "border-warning/40 text-warning",
  project: "border-success/40 text-success",
  reference: "border-border text-ink-muted",
};

/** Top-bar control: a button that opens the memory viewer (project + global). */
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
        aria-label={open ? "Hide memory" : "Show memory"}
        title="Memory (what the agent remembers)"
        className={cn(
          "grid size-8 place-items-center rounded-lg transition",
          open ? "bg-accent-soft text-accent" : "text-ink-muted hover:bg-accent-subtle hover:text-accent",
        )}
      >
        <NotebookText className="size-4" />
      </button>
      {/* Instant show/hide — no fade (avoids WKWebView half-opacity flicker). */}
      {open && <MemoryPopover />}
    </div>
  );
}

function MemoryPopover() {
  const setOpen = useSessionStore((s) => s.setMemoryViewerOpen);
  const loading = useSessionStore((s) => s.loadingMemory);
  const project = useSessionStore((s) => s.memoryOverview);
  const global = useSessionStore((s) => s.globalMemoryOverview);
  const reload = useSessionStore((s) => s.loadMemory);
  const enabled = useSessionStore((s) => s.memoriesEnabled);
  const cwd = useSessionStore((s) => s.localSettings.cwd);
  const status = useSessionStore((s) => s.memoryStatus);

  return (
    <div className="popover-surface absolute right-0 top-10 z-50 flex max-h-[70vh] w-[26rem] flex-col overflow-hidden rounded-xl border border-border bg-bg-elevated shadow-xl">
      <header className="flex items-center gap-2 border-b border-border-subtle px-3 py-2.5">
        <NotebookText className="size-4 shrink-0 text-ink-muted" />
        <div className="min-w-0">
          <p className="text-sm font-medium text-ink">Memory</p>
          <p className="text-xs text-ink-faint">
            {enabled ? "What the agent remembers, per project and globally" : "Currently off"}
          </p>
        </div>
        <button
          onClick={() => void reload()}
          disabled={loading || !enabled}
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
        {!enabled ? (
          <div className="py-6 text-center">
            <p className="text-sm font-medium text-ink-secondary">Memories are off</p>
            <p className="mx-auto mt-1 max-w-[20rem] text-xs text-ink-muted">
              Turn on <span className="font-medium text-ink-secondary">Enable memories</span> in your
              profile menu to let the agent remember durable facts across chats.
            </p>
          </div>
        ) : loading && !project && !global ? (
          <p className="py-6 text-center text-xs text-ink-faint">Reading memory…</p>
        ) : status && !project && !global ? (
          // A failed read is not the same as "no memories yet" — say so.
          <p className="py-6 text-center text-xs text-danger">{status}</p>
        ) : (
          <div className="space-y-4">
            <Scope
              icon={<FolderGit2 className="size-3" />}
              label={cwd ? `Project · ${projectName(cwd)}` : "Project"}
              overview={project}
              empty={cwd ? "Nothing saved for this project yet." : "Choose a project folder to see its memory."}
            />
            <Scope
              icon={<Globe className="size-3" />}
              label="Global · all projects"
              overview={global}
              empty="Nothing saved globally yet."
            />
          </div>
        )}
      </div>

      <footer className="border-t border-border-subtle px-3 py-2 text-xs text-ink-faint">
        The agent curates memory itself with its <code className="rounded bg-chip px-1 py-px font-mono">memory</code> tool.
      </footer>
    </div>
  );
}

/** One scope (project or global): its index + fact cards, or an empty hint. */
function Scope({
  icon,
  label,
  overview,
  empty,
}: {
  icon: ReactNode;
  label: string;
  overview: MemoryOverview | null;
  empty: string;
}) {
  const hasContent = overview && (overview.index || overview.facts.length > 0);
  return (
    <section>
      <SectionLabel icon={icon} label={label} />
      {!hasContent ? (
        <p className="rounded-lg border border-dashed border-border-subtle px-3 py-2.5 text-xs text-ink-faint">
          {empty}
        </p>
      ) : (
        <div className="space-y-2">
          {overview?.index && (
            <div className="rounded-lg border border-border-subtle bg-bg-sunken/60 px-3 py-2">
              <div className={MD}>
                <Markdown remarkPlugins={[remarkGfm]}>{overview.index}</Markdown>
              </div>
            </div>
          )}
          {overview?.facts.map((f) => (
            <FactCard key={f.file} fact={f} />
          ))}
        </div>
      )}
    </section>
  );
}

function SectionLabel({ icon, label }: { icon: ReactNode; label: string }) {
  return (
    <div className="mb-1.5 flex items-center gap-1.5 text-xs font-medium uppercase tracking-wider text-ink-faint">
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
        <FileText className="size-3 shrink-0 text-ink-faint" />
        <span className="min-w-0 flex-1 truncate text-xs font-medium text-ink-secondary">
          {fact.name || fact.description || fact.file}
        </span>
        {fact.kind && (
          <span
            className={cn("shrink-0 rounded-full border px-1.5 py-px text-xs font-medium", tone)}
          >
            {fact.kind}
          </span>
        )}
      </summary>
      {fact.description && fact.name && (
        <p className="mt-1 text-xs text-ink-muted">{fact.description}</p>
      )}
      {fact.body && (
        <div className={cn(MD, "mt-1.5 border-t border-border-subtle pt-1.5")}>
          <Markdown remarkPlugins={[remarkGfm]}>{fact.body}</Markdown>
        </div>
      )}
      <p className="mt-1.5 font-mono text-xs text-ink-faint">{fact.file}</p>
    </details>
  );
}
