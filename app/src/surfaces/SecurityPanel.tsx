import { useCallback, useEffect, useRef, useState } from "react";
import { Download, RefreshCw, ShieldCheck, X } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { saveSecurityScanPdf } from "../lib/securityReport";
import { projectName } from "../lib/localAgent";
import { cn } from "../lib/cn";
import { codeKeyAccountBinding } from "../lib/account";
import type {
  SecurityScanRecord,
  SecuritySeverity,
} from "../core-bridge/types";

const SEVERITY_TONE: Record<SecuritySeverity, string> = {
  critical: "border-danger/50 text-danger",
  high: "border-warning/50 text-warning",
  medium: "border-info/50 text-info",
  low: "border-border text-ink-muted",
};

export function summarizeSecurityScan(record: SecurityScanRecord) {
  return {
    sealed: Boolean(record.seal),
    findings: record.seal?.findings.length ?? 0,
    reviewed: record.seal?.reviewedFiles ?? record.bundle.coverage.length,
    excluded:
      record.seal?.excludedFiles
      ?? record.bundle.coverage.filter((row) => row.status === "excluded").length,
    supporting:
      record.seal?.supportingFiles ?? record.bundle.supportingCoverage.length,
  };
}

export function SecurityButton() {
  const bridge = useSessionStore((state) => state.bridge);
  const auth = useSessionStore((state) => state.auth);
  const cwd = useSessionStore(
    (state) => state.activeProjectRoot ?? state.localSettings.cwd,
  );
  const accountScope = codeKeyAccountBinding(auth);
  const [open, setOpen] = useState(false);
  const [records, setRecords] = useState<SecurityScanRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const contextKey = `${accountScope ?? "signed-out"}\u0000${cwd.trim()}`;
  const contextRef = useRef(contextKey);
  contextRef.current = contextKey;

  useEffect(() => {
    setOpen(false);
    setRecords([]);
    setLoading(false);
    setError(null);
  }, [contextKey]);

  const load = useCallback(async () => {
    const activeBridge = bridge;
    if (!cwd || !activeBridge?.listSecurityScans) return;
    const requestContext = contextKey;
    setLoading(true);
    setError(null);
    try {
      const nextRecords = await activeBridge.listSecurityScans(cwd);
      if (contextRef.current === requestContext) setRecords(nextRecords);
    } catch (cause) {
      if (contextRef.current === requestContext) {
        setError(cause instanceof Error ? cause.message : String(cause));
      }
    } finally {
      if (contextRef.current === requestContext) setLoading(false);
    }
  }, [bridge, contextKey, cwd]);

  useEffect(() => {
    if (open) {
      void load();
    }
  }, [load, open]);

  useEffect(() => {
    if (!open) return;
    const onDown = (event: MouseEvent) => {
      if (!wrapRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={wrapRef} className="relative">
      <button
        onClick={() => setOpen((current) => !current)}
        aria-label={open ? "Hide Security scans" : "Show Security scans"}
        title="Security scan history"
        className={cn(
          "grid size-8 place-items-center rounded-lg transition",
          open
            ? "bg-accent-soft text-accent"
            : "text-ink-muted hover:bg-accent-subtle hover:text-accent",
        )}
      >
        <ShieldCheck className="size-4" />
      </button>
      {open && (
        <SecurityPopover
          cwd={cwd}
          records={records}
          loading={loading}
          error={error}
          onReload={load}
          onClose={() => setOpen(false)}
        />
      )}
    </div>
  );
}

function SecurityPopover({
  cwd,
  records,
  loading,
  error,
  onReload,
  onClose,
}: {
  cwd: string;
  records: SecurityScanRecord[];
  loading: boolean;
  error: string | null;
  onReload: () => Promise<void>;
  onClose: () => void;
}) {
  return (
    <div className="popover-surface absolute right-0 top-10 z-50 flex max-h-[72vh] w-[30rem] flex-col overflow-hidden rounded-xl border border-border bg-bg-elevated shadow-xl">
      <header className="flex items-center gap-2 border-b border-border-subtle px-3 py-2.5">
        <ShieldCheck className="size-4 shrink-0 text-ink-muted" />
        <div className="min-w-0">
          <p className="text-sm font-medium text-ink">Security</p>
          <p className="truncate text-xs text-ink-faint">
            {cwd ? projectName(cwd) : "No project selected"}
          </p>
        </div>
        <button
          onClick={() => void onReload()}
          disabled={loading}
          aria-label="Reload Security scans"
          className="ml-auto grid size-7 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:opacity-50"
        >
          <RefreshCw
            className={cn(
              "size-3.5",
              loading && "animate-[spin_1s_linear_infinite]",
            )}
          />
        </button>
        <button
          onClick={onClose}
          aria-label="Close"
          className="grid size-7 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink"
        >
          <X className="size-3.5" />
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-3">
        {error ? (
          <p className="py-6 text-center text-xs text-danger">{error}</p>
        ) : loading && records.length === 0 ? (
          <p className="py-6 text-center text-xs text-ink-faint">
            Reading Security artifacts…
          </p>
        ) : records.length === 0 ? (
          <div className="rounded-lg border border-dashed border-border-subtle px-4 py-5 text-center">
            <p className="text-sm font-medium text-ink-secondary">No scans yet</p>
            <p className="mt-1 text-xs text-ink-muted">
              Run <code>/security</code>,{" "}
              <code>/security-diff</code>, or{" "}
              <code>/security-deep</code>.
            </p>
          </div>
        ) : (
          <div className="space-y-2">
            {records.map((record) => (
              <ScanCard key={`${record.path}:${record.seal?.bundleDigest ?? "open"}`} record={record} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function ScanCard({ record }: { record: SecurityScanRecord }) {
  const summary = summarizeSecurityScan(record);
  const [saving, setSaving] = useState(false);
  const [exportError, setExportError] = useState<string | null>(null);
  const save = async () => {
    setSaving(true);
    setExportError(null);
    try {
      await saveSecurityScanPdf(record);
    } catch (cause) {
      setExportError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(false);
    }
  };
  return (
    <details className="group rounded-lg border border-border-subtle bg-bg-sunken/40 px-3 py-2.5">
      <summary className="cursor-pointer list-none">
        <div className="flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-xs font-semibold text-ink-secondary">
            {record.bundle.scanId}
          </span>
          <span className="rounded-full border border-border px-1.5 py-px text-xs uppercase text-ink-muted">
            {record.bundle.mode}
          </span>
          <span
            className={cn(
              "rounded-full border px-1.5 py-px text-xs",
              summary.sealed
                ? "border-success/40 text-success"
                : "border-warning/40 text-warning",
            )}
          >
            {summary.sealed ? "sealed" : "in progress"}
          </span>
        </div>
        <p className="mt-1 text-xs text-ink-faint">
          {summary.findings} findings · {summary.reviewed} reviewed ·{" "}
          {summary.excluded} excluded
          {summary.supporting > 0 ? ` · ${summary.supporting} supporting` : ""}
          {record.seal?.deepPasses
            ? ` · ${record.seal.deepPasses} deep passes`
            : ""}
        </p>
      </summary>

      <button type="button" disabled={saving} onClick={() => void save()}
        className="mt-2 flex items-center gap-1 text-xs text-ink-muted hover:text-ink disabled:opacity-50">
        <Download className="size-3" /> {saving ? "Saving…" : "Save as PDF"}
      </button>
      {exportError && <p role="alert" className="mt-1 text-xs text-danger">{exportError}</p>}
      {record.seal?.findings.length ? (
        <div className="mt-2 space-y-1.5 border-t border-border-subtle pt-2">
          {record.seal.findings.map((finding) => (
            <div key={finding.findingId} className="rounded-md border border-border-subtle p-2">
              <div className="flex items-center gap-2">
                <span className="font-mono text-xs text-ink-faint">
                  {finding.findingId}
                </span>
                <span
                  className={cn(
                    "ml-auto rounded-full border px-1.5 py-px text-xs uppercase",
                    SEVERITY_TONE[finding.severity],
                  )}
                >
                  {finding.severity}
                </span>
              </div>
              <p className="mt-1 text-xs text-ink-secondary">{finding.impact}</p>
              <p className="mt-1 truncate font-mono text-xs text-ink-faint">
                {finding.sourcePath}
              </p>
            </div>
          ))}
        </div>
      ) : null}
      <p className="mt-2 truncate border-t border-border-subtle pt-2 font-mono text-xs text-ink-faint">
        {record.path}
      </p>
    </details>
  );
}
