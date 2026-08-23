import { memo } from "react";
import { X } from "lucide-react";
import { cn } from "../../lib/cn";
import { sameContentBlocks } from "../../lib/contentBlocks";
import { specProgressTitle, specTrailWindow } from "../../lib/specProgress";
import { STATUS_LABEL, TOOL_KIND_LABEL } from "../../lib/toolLabels";
import { ProgressIcon, TOOL_KIND_ICON } from "../toolPresentation";
import { ResearchOutline } from "../work/ResearchWork";
import type { ToolCall } from "../../core-bridge/types";

function callName(call: ToolCall): string {
  return specProgressTitle(call) ?? TOOL_KIND_LABEL[call.kind];
}

function chipLabel(call: ToolCall): string {
  return `${TOOL_KIND_LABEL[call.kind]}: ${callName(call)} — ${STATUS_LABEL[call.status]}`;
}

/** Snapshots are re-cloned every streamed token, so compare only what these
 *  components read. */
function sameCalls(a: readonly ToolCall[], b: readonly ToolCall[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((call, index) => {
    const other = b[index];
    return call.id === other.id
      && call.status === other.status
      && call.kind === other.kind
      && call.title === other.title
      && call.progress?.revision === other.progress?.revision
      && call.locations.length === other.locations.length
      && sameContentBlocks(call.content, other.content);
  });
}

/** One tool call as a glyph. Status is carried by shape and an accessible name
 *  before colour: `html.colorblind` swaps success/danger to blue/orange, so a
 *  failure changes its glyph rather than only its hue. The label beside the card's
 *  dot already says what is running, so a chip carries no visible text.
 *
 *  Only the running chip is boxed. A row of bordered squares would read as a
 *  toolbar — `bg-chip` is this codebase's interactive affordance — so finished and
 *  queued work stays a bare mark and the single tinted pill says "you are here". */
function TrailChipImpl({ call }: { call: ToolCall }) {
  const running = call.status === "in_progress";
  const Kind = TOOL_KIND_ICON[call.kind];

  return (
    <span
      aria-label={chipLabel(call)}
      title={chipLabel(call)}
      className={cn(
        "grid shrink-0 place-items-center",
        running ? "size-5 rounded-md bg-accent-subtle text-accent ring-1 ring-accent/25" : "size-4",
        call.status === "completed" && "text-ink-muted",
        call.status === "pending" && "text-ink-faint/55",
        call.status === "failed" && "text-danger",
        call.status === "cancelled" && "text-ink-faint/55",
      )}
    >
      {call.status === "failed" ? (
        <ProgressIcon status="failed" className="size-3.5" />
      ) : call.status === "cancelled" ? (
        <X aria-hidden className="size-3" />
      ) : (
        <Kind aria-hidden className={cn("size-3.5", running && "breathe")} />
      )}
    </span>
  );
}

const TrailChip = memo(
  TrailChipImpl,
  (a, b) =>
    a.call.id === b.call.id
    && a.call.status === b.call.status
    && a.call.kind === b.call.kind
    && a.call.title === b.call.title,
);

/** The strip: one glyph per tool call of the turn, in timeline order. Chips mount
 *  settled — a single 4px glyph appearing does not deserve choreography, and
 *  staggering it would invite the delay arithmetic motionPolicy bans. */
function SpecToolTrailImpl({ calls }: { calls: readonly ToolCall[] }) {
  if (calls.length === 0) return null;
  const { hidden, visible } = specTrailWindow(calls);

  return (
    <div className="flex min-w-0 items-center gap-2" aria-label="Tools used in this run">
      {hidden > 0 && (
        <span className="shrink-0 text-xs tabular-nums text-ink-faint/70">+{hidden}</span>
      )}
      {visible.map((call) => <TrailChip key={call.id} call={call} />)}
    </div>
  );
}

export const SpecToolTrail = memo(SpecToolTrailImpl, (a, b) => sameCalls(a.calls, b.calls));

/** The expanded view: one row per call, with the delegated-progress outline
 *  nested under any call that actually reports one. */
function SpecToolListImpl({ calls }: { calls: readonly ToolCall[] }) {
  if (calls.length === 0) return null;

  return (
    <ul className="space-y-1">
      {calls.map((call) => {
        const path = call.locations?.[0]?.path;
        return (
          <li key={call.id} className="text-sm">
            <div className="grid min-h-7 grid-cols-[1rem_4.5rem_minmax(0,1fr)_auto] items-center gap-2">
              <ProgressIcon status={call.status} className="size-3.5" />
              <span className="truncate text-xs font-medium text-ink-faint">
                {TOOL_KIND_LABEL[call.kind]}
              </span>
              <span className={cn("min-w-0 truncate text-ink-secondary", path && "font-mono text-xs")}>
                {path ?? callName(call)}
              </span>
              <span className="pl-3 text-xs text-ink-faint">{STATUS_LABEL[call.status]}</span>
            </div>
            {/* Gated on `progress`: ResearchOutline shows a spinner and "Starting
                research agent" whenever it is absent, whatever the call's status. */}
            {call.progress && (
              <div className="ml-[1.6rem] border-l border-border-subtle pl-3">
                <ResearchOutline progress={call.progress} />
              </div>
            )}
          </li>
        );
      })}
    </ul>
  );
}

export const SpecToolList = memo(SpecToolListImpl, (a, b) => sameCalls(a.calls, b.calls));
