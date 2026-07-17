import { cn } from "../../lib/cn";

/** Shared layout primitives for settings surfaces (Settings dialog, MCP/SSH
 *  dialogs, organization knowledge). One set of paddings, gaps, and text sizes
 *  so labels, descriptions, and controls sit on the same baseline grid. */

export function GroupLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="mb-2 text-xs font-semibold uppercase tracking-wider text-ink-faint">
      {children}
    </div>
  );
}

export function Card({ children }: { children: React.ReactNode }) {
  return (
    <div className="overflow-hidden rounded-xl border border-border-subtle bg-bg-elevated/40 [&>*+*]:border-t [&>*+*]:border-border-subtle">
      {children}
    </div>
  );
}

/** A standard settings row: optional leading icon, a name (with optional
 *  description beneath), then trailing controls. Icon and trailing controls
 *  center against the whole row; long descriptions don't push them down. */
export function Row({
  icon,
  name,
  sub,
  children,
}: {
  icon?: React.ReactNode;
  name: React.ReactNode;
  sub?: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-3 px-3.5 py-3">
      {icon && <span className="grid size-4 shrink-0 place-items-center text-ink-muted">{icon}</span>}
      <div className="min-w-0 flex-1">
        <div className="text-sm leading-5 text-ink">{name}</div>
        {sub && <div className="mt-0.5 text-xs leading-4 text-ink-faint">{sub}</div>}
      </div>
      {children}
    </div>
  );
}

export function Toggle({ on, onClick, label }: { on: boolean; onClick: () => void; label: string }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      onClick={onClick}
      className={cn(
        "relative h-[18px] w-8 shrink-0 rounded-full transition-colors",
        on ? "bg-accent" : "bg-bg-tertiary",
      )}
    >
      <span
        className={cn(
          "absolute top-0.5 size-[14px] rounded-full bg-white shadow-sm transition-all",
          on ? "left-[15px]" : "left-0.5",
        )}
      />
    </button>
  );
}
