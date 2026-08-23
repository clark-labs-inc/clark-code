import type { ToolKind, ToolStatus } from "../core-bridge/types";

/** Human names for tool kinds and statuses. Lives in `lib/` (which stays
 *  JSX- and lucide-free) so pure derivations can name a tool without pulling in
 *  the icon vocabulary. `surfaces/toolPresentation.tsx` re-exports both maps
 *  alongside the glyphs, so a rendering surface needs only one import. */
export const TOOL_KIND_LABEL: Record<ToolKind, string> = {
  read: "Read", edit: "Edit", delete: "Delete", move: "Move",
  search: "Search", execute: "Command", think: "Think", fetch: "Fetch",
  research: "Research", view_image: "Image", generate_image: "Generate",
  other: "Tool",
};

export const STATUS_LABEL: Record<ToolStatus, string> = {
  pending: "Pending",
  in_progress: "Running",
  completed: "Complete",
  cancelled: "Cancelled",
  failed: "Failed",
};
