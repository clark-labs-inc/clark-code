import {
  BookOpen,
  Brain,
  Circle,
  CircleCheck,
  Download,
  Eye,
  FolderInput,
  Globe2,
  ImagePlus,
  Loader2,
  PencilLine,
  Search,
  SquareTerminal,
  Trash2,
  TriangleAlert,
  Wrench,
  X,
} from "lucide-react";
import { cn } from "../lib/cn";
import type { ToolKind, ToolStatus } from "../core-bridge/types";

/** Shared vocabulary for surfaces that summarize a run's tool calls as a trail
 *  of glyphs, such as specialist research presentations.
 *
 *  `surfaces/work/WorkLine.tsx` must never import TOOL_KIND_ICON. The dense chat
 *  row deliberately carries no per-kind iconography ("activity icon restraint",
 *  enforced by `surfaces/activityIcons.spec.tsx`): there, a leading glyph on
 *  every row is noise because the row already spells out the verb and target. A
 *  trail is the opposite case — the glyph *is* the whole row. */
export const TOOL_KIND_ICON: Record<ToolKind, typeof Search> = {
  read: BookOpen, edit: PencilLine, delete: Trash2, move: FolderInput,
  search: Search, execute: SquareTerminal, think: Brain, fetch: Download,
  // Matches the Globe2 that already heads a research row in ResearchWork, so a
  // trail pip and its chat row read as the same thing.
  research: Globe2,
  view_image: Eye, generate_image: ImagePlus, other: Wrench,
};

export const STATUS_TEXT: Record<ToolStatus, string> = {
  pending: "text-ink-faint",
  in_progress: "text-accent",
  completed: "text-success",
  cancelled: "text-ink-faint",
  failed: "text-danger",
};

export function ProgressIcon({
  status,
  className,
}: {
  status: ToolStatus;
  className?: string;
}) {
  if (status === "completed") {
    return <CircleCheck aria-hidden className={cn("text-success", className)} />;
  }
  if (status === "in_progress") {
    return (
      <Loader2
        aria-hidden
        className={cn("animate-[spin_1s_linear_infinite] text-accent", className)}
      />
    );
  }
  if (status === "failed") {
    return <TriangleAlert aria-hidden className={cn("text-danger", className)} />;
  }
  if (status === "cancelled") {
    return <X aria-hidden className={cn("text-ink-faint", className)} />;
  }
  return <Circle aria-hidden className={cn("text-ink-faint", className)} />;
}
