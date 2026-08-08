// The Changes panel — a PR-style review of everything that changed since the
// conversation's first checkpoint: per-file +/- stats, expandable unified
// diffs, and per-file revert. Local git sessions only (the baseline is a
// checkpoint commit; local and remote sessions share the executor-backed path
// in provider-local/src/changes.rs.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  GitCompareArrows, X, RefreshCw, ChevronRight, Undo2, FilePlus2, FileMinus2, FilePen,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import type { RunView } from "../core-bridge/types";
import type { RemoteInfo } from "../lib/remoteWorker";
import { DiffBody } from "./work/WorkLine";
import { cn } from "../lib/cn";
import { minLoadDuration } from "../lib/minLoadDuration";

export interface ChangedFile {
  path: string;
  previous_path?: string;
  additions: number;
  deletions: number;
  status: string;
}

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Every run's checkpoint, in chronological order (runs merge in timeline
 *  order, so restored history comes first). The default baseline is the
 *  FIRST one (the conversation's start — unchanged default), but the popover
 *  lets the user pick any later one to see what changed since a specific turn.
 *
 *  The selector returns a stable STRING signature of the checkpoints (a
 *  primitive, value-compared) — NOT the `runs` object, whose reference changes
 *  on every streamed snapshot and would re-render this always-mounted top-bar
 *  control on every token. Deriving the array happens in a `useMemo` keyed off
 *  that signature; a selector that returned a freshly-built array each call
 *  would break `useSyncExternalStore`'s reference check (infinite render loop). */
function useCheckpointedRuns(): string[] {
  const signature = useSessionStore((s) => {
    const runs = s.snapshot.runs as Record<string, RunView>;
    return Object.values(runs)
      .map((run) => run.checkpoint)
      .filter((c): c is string => !!c)
      .join("\n");
  });
  return useMemo(() => (signature ? signature.split("\n") : []), [signature]);
}

const STATUS_ICON: Record<string, typeof FilePen> = {
  added: FilePlus2,
  deleted: FileMinus2,
  modified: FilePen,
  renamed: FilePen,
};

type RemoteWorkerArg = Pick<RemoteInfo, "id">;

export function changesBaseForRuns(current: string | undefined, runs: string[]): string {
  return current && runs.includes(current) ? current : runs[0];
}

function remoteWorkerArg(remote: RemoteInfo | null): RemoteWorkerArg | null {
  return remote ? { id: remote.id } : null;
}

/** Top-bar control: opens the session-changes review popover. */
export function ChangesButton() {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const runs = useCheckpointedRuns();
  const sessionId = useSessionStore((s) => s.session?.id ?? null);

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
  }, [open]);

  if (runs.length === 0 || !isTauri()) return null;
  return (
    <div ref={wrapRef} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        aria-label={open ? "Hide changes" : "Show changes"}
        title="Changes this conversation made (review & revert)"
        className={cn(
          "grid size-8 place-items-center rounded-lg transition",
          open ? "bg-accent-soft text-accent" : "text-ink-muted hover:bg-accent-subtle hover:text-accent",
        )}
      >
        <GitCompareArrows className="size-4" />
      </button>
      {/* Instant show/hide — no fade (avoids WKWebView half-opacity flicker). */}
      {open && <ChangesPopover key={sessionId} runs={runs} onClose={() => setOpen(false)} />}
    </div>
  );
}

function ChangesPopover({ runs, onClose }: { runs: string[]; onClose: () => void }) {
  const cwd = useSessionStore((s) => s.activeProjectRoot ?? "");
  const activeRemote = useSessionStore((s) => s.activeRemote);
  const remote = useMemo(() => remoteWorkerArg(activeRemote), [activeRemote]);
  // Default baseline: the conversation's first checkpoint (unchanged default
  // behavior) — the picker below lets the user diff against any later turn.
  const [base, setBase] = useState(runs[0]);
  const [files, setFiles] = useState<ChangedFile[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    setBase((current) => changesBaseForRuns(current, runs));
    setFiles(null);
  }, [runs]);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      // minLoadDuration holds the spinner for one spin — `git diff --stat` over
      // a small tree resolves in a single frame and React never paints the
      // spinning state, so the refresh click looks frozen.
      const files = await minLoadDuration(invoke<ChangedFile[]>("changes_summary", { cwd, base, remote }));
      setFiles(files);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [cwd, base, remote]);

  useEffect(() => {
    void load();
  }, [load]);

  const totalAdd = (files ?? []).reduce((n, f) => n + f.additions, 0);
  const totalDel = (files ?? []).reduce((n, f) => n + f.deletions, 0);

  return (
    <div className="popover-surface absolute right-0 top-10 z-50 flex max-h-[72vh] w-[30rem] flex-col overflow-hidden rounded-xl border border-border bg-bg-elevated shadow-xl">
      <header className="flex items-center gap-2 border-b border-border-subtle px-3 py-2.5">
        <GitCompareArrows className="size-4 shrink-0 text-ink-muted" />
        <div className="min-w-0">
          <p className="text-sm font-medium text-ink">Changes</p>
          <p className="text-xs text-ink-faint">
            {files && files.length > 0 ? (
              <>
                {files.length} file{files.length === 1 ? "" : "s"} ·{" "}
                <span className="text-success">+{totalAdd}</span>{" "}
                <span className="text-danger">−{totalDel}</span> since the selected point
              </>
            ) : (
              "Everything changed since the selected point, reviewable per file"
            )}
          </p>
        </div>
        {runs.length > 1 && (
          <select
            value={base}
            onChange={(e) => setBase(e.target.value)}
            title="Diff against a different point in this conversation"
            className="shrink-0 rounded-md border border-border bg-bg px-1.5 py-1 text-xs text-ink-secondary outline-none disabled:opacity-50"
          >
            {runs.map((sha, i) => (
              <option key={sha} value={sha}>
                {i === 0 ? "Since start" : `Since turn ${i + 1}`}
              </option>
            ))}
          </select>
        )}
        <button
          onClick={() => void load()}
          disabled={loading}
          title="Refresh"
          aria-label="Refresh changes"
          className="grid size-7 shrink-0 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:opacity-50"
        >
          <RefreshCw className={cn("size-3.5", loading && "animate-[spin_1s_linear_infinite]")} />
        </button>
        <button
          onClick={onClose}
          aria-label="Close"
          className="grid size-7 shrink-0 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink"
        >
          <X className="size-3.5" />
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        {error ? (
          <p className="px-2 py-6 text-center text-xs text-danger">{error}</p>
        ) : files === null ? (
          <p className="px-2 py-6 text-center text-xs text-ink-faint">Comparing…</p>
        ) : files.length === 0 ? (
          <p className="px-2 py-6 text-center text-xs text-ink-faint">
            No changes yet — the working tree matches where this conversation started.
          </p>
        ) : (
          <div className="flex flex-col gap-1">
            {files.map((f) => (
              <FileRow
                key={f.path}
                file={f}
                cwd={cwd}
                base={base}
                remote={remote}
                onReverted={() => void load()}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function FileRow({
  file,
  cwd,
  base,
  remote,
  onReverted,
}: {
  file: ChangedFile;
  cwd: string;
  base: string;
  remote: RemoteWorkerArg | null;
  onReverted: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [diff, setDiff] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const Icon = STATUS_ICON[file.status] ?? FilePen;

  const toggle = async () => {
    const next = !open;
    setOpen(next);
    if (next && diff === null) {
      try {
        setDiff(await invoke<string>("changes_diff", {
          cwd,
          base,
          path: file.path,
          previousPath: file.previous_path ?? null,
          remote,
        }));
      } catch (e) {
        setDiff(`(diff unavailable: ${String(e)})`);
      }
    }
  };

  const revert = async () => {
    setBusy(true);
    try {
      await invoke("changes_revert", {
        cwd,
        base,
        path: file.path,
        previousPath: file.previous_path ?? null,
        remote,
      });
      onReverted();
    } catch (e) {
      setDiff(`(revert failed: ${String(e)})`);
      setBusy(false);
      setConfirming(false);
    }
  };

  return (
    <div className="overflow-hidden rounded-lg border border-border-subtle bg-bg-sunken/40">
      <div className="group flex items-center gap-2 px-2.5 py-1.5">
        <button
          onClick={() => void toggle()}
          className="flex min-w-0 flex-1 items-center gap-2 text-left"
          title={file.path}
        >
          <ChevronRight
            className={cn("size-3.5 shrink-0 text-ink-faint transition-transform", open && "rotate-90")}
          />
          <Icon
            className={cn(
              "size-3.5 shrink-0",
              file.status === "added" && "text-success",
              file.status === "deleted" && "text-danger",
              file.status !== "added" && file.status !== "deleted" && "text-ink-muted",
            )}
          />
          <span className="min-w-0 flex-1 truncate font-mono text-xs text-ink-secondary">
            {file.path}
          </span>
        </button>
        <span className="shrink-0 font-mono text-xs tabular-nums">
          {file.additions > 0 && <span className="text-success">+{file.additions}</span>}{" "}
          {file.deletions > 0 && <span className="text-danger">−{file.deletions}</span>}
        </span>
        {confirming ? (
          <span className="flex shrink-0 items-center gap-1">
            <button
              onClick={() => void revert()}
              disabled={busy}
              className="rounded-md bg-danger/12 px-2 py-0.5 text-xs font-medium text-danger transition hover:bg-danger/20 disabled:opacity-50"
            >
              {busy ? "Reverting…" : "Revert file"}
            </button>
            <button
              onClick={() => setConfirming(false)}
              disabled={busy}
              className="rounded-md px-1.5 py-0.5 text-xs text-ink-muted transition hover:bg-bg-hover"
            >
              Keep
            </button>
          </span>
        ) : (
          <button
            onClick={() => setConfirming(true)}
            title={file.status === "added" ? "Delete this created file" : "Restore to how it was when the conversation started"}
            aria-label={`Revert ${file.path}`}
            className="grid size-6 shrink-0 place-items-center rounded-md text-ink-faint opacity-0 transition hover:bg-bg-hover hover:text-danger group-hover:opacity-100"
          >
            <Undo2 className="size-3.5" />
          </button>
        )}
      </div>
      {open && diff !== null && (
        <div className="max-h-72 overflow-y-auto border-t border-border-subtle">
          <DiffBody text={diff} />
        </div>
      )}
    </div>
  );
}
