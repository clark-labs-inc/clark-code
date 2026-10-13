import type { ToolStatus } from "../core-bridge/types";

/** Shared status labels for specialist research presentation. */
export const STATUS_LABEL: Record<ToolStatus, string> = {
  pending: "Pending",
  in_progress: "Running",
  completed: "Complete",
  cancelled: "Cancelled",
  failed: "Failed",
};
