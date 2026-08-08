import { useEffect, useRef, useState } from "react";
import { Activity, Circle, GitMerge, UsersRound } from "lucide-react";
import type { ProjectActivity } from "../core-bridge/bridge";
import { cn } from "../lib/cn";

export interface DesktopPeerActivity {
  id: string;
  title: string;
}

interface ParallelWorkContextProps {
  activity: ProjectActivity;
  branch: string;
  desktopPeers: DesktopPeerActivity[];
  onOpenPeer?: (id: string) => void;
}

const ITEM =
  "flex min-h-7 min-w-0 items-center gap-1 rounded-md bg-composer-context px-1.5 text-xs font-medium leading-none";

export function activityAge(updatedAtMs: number, now = Date.now()): string {
  const seconds = Math.max(0, Math.round((now - updatedAtMs) / 1_000));
  if (seconds < 45) return "active now";
  const minutes = Math.max(1, Math.round(seconds / 60));
  return `${minutes}m ago`;
}

export function ParallelWorkContext({
  activity,
  branch,
  desktopPeers,
  onOpenPeer,
}: ParallelWorkContextProps) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const external = activity.externalAgents;
  const agentCount = external.length + desktopPeers.length;
  const workingFiles = activity.changedFiles + activity.untrackedFiles + activity.conflictedFiles;

  useEffect(() => {
    if (!open) return;
    const closeOutside = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) setOpen(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", closeOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [open]);

  return (
    <div ref={ref} className="relative shrink-0">
      <button
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={agentCount > 0 ? `${agentCount} other agent${agentCount === 1 ? "" : "s"} active in this checkout` : "No other agents detected in this checkout"}
        onClick={() => setOpen((current) => !current)}
        className={cn(
          ITEM,
          "transition hover:bg-bg-hover",
          agentCount > 0 ? "text-accent" : "text-ink-faint",
        )}
      >
        <UsersRound className="size-3 shrink-0" />
        <span>{agentCount > 0 ? `${agentCount} other${agentCount === 1 ? "" : "s"}` : "No peers"}</span>
        <Circle
          className={cn(
            "size-1.5 fill-current",
            agentCount > 0 ? "text-success" : "text-ink-faint",
          )}
        />
      </button>

      {open && (
        <div
          role="dialog"
          aria-label="Other agents in this checkout"
          // On a phone the chip can sit near the right edge of a wrapped row;
          // right-aligning a viewport-sized popover then clips its left side.
          // Anchor it to the chip on narrow screens and keep the desktop
          // right alignment where the composer has room.
          className="popover-surface absolute bottom-full left-1/2 z-50 mb-2 w-[min(22rem,calc(100vw-6rem))] -translate-x-1/2 overflow-hidden rounded-2xl bg-bg-elevated shadow-lifted ring-1 ring-border-subtle sm:left-auto sm:right-0 sm:translate-x-0"
        >
          <div className="border-b border-border-subtle px-3.5 py-3">
            <div className="flex items-center gap-2">
              <span className="grid size-7 place-items-center rounded-lg bg-accent-subtle text-accent">
                <UsersRound className="size-4" />
              </span>
              <div className="min-w-0">
                <p className="text-sm font-semibold text-ink">Other agents in this checkout</p>
                <p className="truncate text-xs text-ink-muted">{branch} · shared checkout</p>
              </div>
            </div>
          </div>

          <div className="grid grid-cols-3 gap-px bg-border-subtle">
            <Stat value={agentCount} label="other agents" active={agentCount > 0} />
            <Stat value={activity.changedFiles} label="tracked" />
            <Stat value={activity.untrackedFiles} label="untracked" />
          </div>

          <div className="max-h-52 overflow-y-auto p-2">
            {desktopPeers.map((peer) => (
              <AgentRow
                key={`desktop:${peer.id}`}
                label="the agent"
                title={peer.title || "Untitled the agent task"}
                status="running now"
                onOpen={onOpenPeer ? () => onOpenPeer(peer.id) : undefined}
              />
            ))}
            {external.map((agent) => (
              <AgentRow
                key={`external:${agent.id}`}
                label="External agent"
                title={agent.title || "Untitled external task"}
                status={activityAge(agent.updatedAtMs)}
              />
            ))}
            {agentCount === 0 && (
              <div className="px-2 py-4 text-center">
                <p className="text-sm text-ink-secondary">No other agent activity detected</p>
                <p className="mt-1 text-xs text-ink-faint">This checkout looks clear for focused work.</p>
              </div>
            )}
          </div>

          <div className="border-t border-border-subtle px-3.5 py-2.5 text-xs leading-relaxed text-ink-faint">
            <p className="flex items-start gap-1.5">
              {activity.conflictedFiles > 0 ? (
                <GitMerge className="mt-0.5 size-3 shrink-0 text-warning" />
              ) : (
                <Activity className="mt-0.5 size-3 shrink-0" />
              )}
              <span>
                {activity.conflictedFiles > 0
                  ? `${activity.conflictedFiles} conflicted file${activity.conflictedFiles === 1 ? "" : "s"}. `
                  : workingFiles > 0
                    ? `${workingFiles} working-tree file${workingFiles === 1 ? "" : "s"}. `
                    : "Working tree clean. "}
                {desktopPeers.length > 0 && external.length > 0
                  ? "the agent sessions are tracked on this device; external activity is inferred from tasks updated in the last five minutes."
                  : desktopPeers.length > 0
                    ? "the agent session activity is tracked on this device."
                    : "External activity is inferred from tasks updated in the last five minutes."}
              </span>
            </p>
          </div>
        </div>
      )}
    </div>
  );
}

function Stat({ value, label, active = false }: { value: number; label: string; active?: boolean }) {
  return (
    <div className="bg-bg-elevated px-3 py-2 text-center">
      <div className={cn("text-sm font-semibold tabular-nums", active ? "text-accent" : "text-ink")}>{value}</div>
      <div className="text-xs uppercase tracking-wide text-ink-faint">{label}</div>
    </div>
  );
}

function AgentRow({
  label,
  title,
  status,
  onOpen,
}: {
  label: string;
  title: string;
  status: string;
  onOpen?: () => void;
}) {
  return (
    <div className="flex items-start gap-2.5 rounded-xl px-2 py-2 hover:bg-bg-hover">
      <span className="relative mt-0.5 grid aspect-square min-h-7 min-w-7 shrink-0 place-items-center rounded-full bg-accent-subtle text-xs font-semibold uppercase text-accent">
        {label.slice(0, 1)}
        <span className="absolute -bottom-0.5 -right-0.5 size-2 rounded-full border-2 border-bg-elevated bg-success" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-1.5">
          <span className="text-xs font-medium text-ink-secondary">{label}</span>
          <span className="text-xs text-ink-faint">{status}</span>
        </span>
        <span className="mt-0.5 block truncate text-xs text-ink-muted" title={title}>{title}</span>
      </span>
      {onOpen && (
        <button
          type="button"
          onClick={onOpen}
          className="shrink-0 rounded-md px-1.5 py-1 text-xs font-medium text-accent transition hover:bg-accent-subtle"
          aria-label={`Open the agent session: ${title}`}
        >
          Open
        </button>
      )}
    </div>
  );
}
