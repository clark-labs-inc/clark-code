import { cn } from "../../lib/cn";

/** Shared layout primitives for settings surfaces (Settings dialog, MCP/SSH
 *  dialogs, organization knowledge). One set of paddings, gaps, and text sizes
 *  so labels, descriptions, and controls sit on the same baseline grid. */

export function GroupLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="mb-2 px-0.5 text-xs font-medium text-ink-secondary">
      {children}
    </div>
  );
}

export function Card({ children }: { children: React.ReactNode }) {
  return (
    <div className="overflow-hidden rounded-xl bg-bg-secondary/55 p-1 [&>*]:rounded-lg">
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
    <div className="flex min-h-14 flex-wrap items-center gap-x-3 gap-y-2 px-3.5 py-2.5">
      {icon && <span className="grid size-4 shrink-0 place-items-center text-ink-muted">{icon}</span>}
      <div className="min-w-0 flex-1">
        <div className="text-sm leading-snug text-ink">{name}</div>
        {sub && <div className="mt-0.5 text-xs leading-snug text-ink-faint">{sub}</div>}
      </div>
      {children && <div className="ml-auto max-w-full shrink-0">{children}</div>}
    </div>
  );
}

export function Toggle({
  on,
  onClick,
  label,
  disabled = false,
}: {
  on: boolean;
  onClick: () => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      onClick={onClick}
      disabled={disabled}
      className="grid size-8 shrink-0 place-items-center rounded-lg outline-none transition hover:bg-bg-hover focus-visible:ring-2 focus-visible:ring-accent/40 disabled:pointer-events-none disabled:opacity-40"
    >
      <span
        className={cn(
          "relative h-[18px] w-8 rounded-full transition-colors",
          on ? "bg-accent" : "bg-bg-tertiary",
        )}
      >
        <span
          className={cn(
            "absolute left-0.5 top-0.5 size-[14px] rounded-full bg-white shadow-sm transition-transform",
            on ? "translate-x-[13px]" : "translate-x-0",
          )}
        />
      </span>
    </button>
  );
}
