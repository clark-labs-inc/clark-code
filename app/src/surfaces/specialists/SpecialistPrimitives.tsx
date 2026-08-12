import type { ReactNode } from "react";
import { AlertTriangle, CheckCircle2, Loader2, RefreshCw } from "lucide-react";
import { cn } from "../../lib/cn";

export function CanvasStatus({
  loading,
  error,
  onRetry,
}: {
  loading: boolean;
  error: string | null;
  onRetry: () => void;
}) {
  if (loading) {
    return (
      <div className="flex min-h-72 items-center justify-center gap-2 text-sm text-ink-muted">
        <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" />
        Loading verified specialist data…
      </div>
    );
  }
  if (!error) return null;
  return (
    <div className="m-5 flex min-h-56 flex-col items-center justify-center border-y border-danger/20 px-8 text-center">
      <AlertTriangle className="mb-3 size-5 text-danger" />
      <div className="text-sm font-semibold text-ink">This view could not be refreshed</div>
      <p className="mt-1 max-w-sm text-xs leading-5 text-ink-muted">{error}</p>
      <button
        type="button"
        onClick={onRetry}
        className="mt-4 flex items-center gap-1.5 rounded-lg bg-bg-secondary px-3 py-1.5 text-xs font-medium text-ink-secondary transition hover:bg-bg-hover"
      >
        <RefreshCw className="size-3.5" /> Try again
      </button>
    </div>
  );
}

export function MetricCard({
  label,
  value,
  detail,
  tone = "default",
}: {
  label: string;
  value: string | number;
  detail?: string;
  tone?: "default" | "good" | "warning" | "danger";
}) {
  return (
    <div className="border-t border-border px-1 py-3">
      <div className="text-xs font-semibold uppercase tracking-[0.08em] text-ink-faint">{label}</div>
      <div className={cn(
        "mt-1 text-2xl font-semibold tracking-[-0.03em] text-ink",
        tone === "good" && "text-success",
        tone === "warning" && "text-warning",
        tone === "danger" && "text-danger",
      )}>
        {value}
      </div>
      {detail && <div className="mt-1 text-xs text-ink-muted">{detail}</div>}
    </div>
  );
}

export function SectionCard({
  title,
  detail,
  action,
  children,
}: {
  title: string;
  detail?: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="border-y border-border-subtle">
      <header className="flex min-h-14 items-center gap-3 px-4 py-3">
        <div className="min-w-0 flex-1">
          <h3 className="text-sm font-semibold text-ink">{title}</h3>
          {detail && <p className="mt-0.5 text-xs text-ink-muted">{detail}</p>}
        </div>
        {action}
      </header>
      {children}
    </section>
  );
}

export function StatusPill({ status }: { status: string }) {
  const normalized = status.toLowerCase();
  const good = ["complete", "completed", "ready", "active", "validated"].includes(normalized);
  const busy = ["running", "queued", "scanning", "in_progress"].includes(normalized);
  const attention = ["failed", "needs_attention", "stale", "blocked"].includes(normalized);
  return (
    <span className={cn(
      "inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium",
      good && "bg-success/10 text-success",
      busy && "bg-accent-soft text-accent",
      attention && "bg-warning/10 text-warning",
      !good && !busy && !attention && "bg-chip text-ink-muted",
    )}>
      {good && <CheckCircle2 className="size-3" />}
      {status.replaceAll("_", " ")}
    </span>
  );
}

export function EmptyState({
  title,
  detail,
}: {
  title: string;
  detail: string;
}) {
  return (
    <div className="px-6 py-12 text-center">
      <div className="text-sm font-medium text-ink-secondary">{title}</div>
      <p className="mx-auto mt-1 max-w-md text-xs leading-5 text-ink-muted">{detail}</p>
    </div>
  );
}
