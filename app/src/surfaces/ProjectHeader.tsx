import { ChevronRight, Folder, GripVertical, MessageSquare, MoreHorizontal, Server, SquarePen } from "lucide-react";
import type { ProjectGroup } from "../lib/projectSidebar";

export function ProjectHeader({
  group,
  expanded,
  conversationPanelId,
  menuOpen,
  reorderable = false,
  dragHandleRef,
  onToggle,
  onOpenMenu,
  onNewSession,
}: {
  group: ProjectGroup;
  expanded: boolean;
  conversationPanelId: string;
  menuOpen: boolean;
  reorderable?: boolean;
  dragHandleRef?: (element: HTMLElement | null) => void;
  onToggle: () => void;
  onOpenMenu: (button: HTMLButtonElement) => void;
  onNewSession: () => void;
}) {
  const Icon = group.kind === "remote" ? Server : group.kind === "local" ? Folder : MessageSquare;
  const canStartSession = Boolean(group.path || group.remoteHost);
  const newSessionLabel =
    group.kind === "remote"
      ? `New session on ${group.label}`
      : `New session in ${group.label}`;
  return (
    <div
      title={group.title}
      className="group mt-2 flex min-h-9 items-center gap-0.5 rounded-lg px-1 text-sm text-ink-secondary hover:bg-bg-hover"
    >
      {reorderable && (
        <span
          ref={dragHandleRef}
          data-project-drag-handle={group.key}
          aria-hidden="true"
          title={`Drag ${group.label} to reorder pinned projects`}
          className="-ml-1 grid size-5 shrink-0 cursor-grab place-items-center rounded-md text-ink-faint opacity-0 transition hover:bg-bg-sunken hover:text-ink group-hover:opacity-100 group-focus-within:opacity-100 active:cursor-grabbing"
        >
          <GripVertical className="size-3.5" />
        </span>
      )}
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={expanded}
        aria-controls={conversationPanelId}
        aria-label={`${expanded ? "Collapse" : "Expand"} ${group.label}`}
        className="relative flex min-w-0 flex-1 items-center gap-2.5 rounded-md px-2 py-1.5 text-left focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent-focus"
      >
        <span className="relative grid size-4 shrink-0 place-items-center text-ink-muted">
          <Icon className="size-4 transition-opacity group-hover:opacity-0 group-focus-within:opacity-0" />
          <ChevronRight aria-hidden="true" className={"absolute size-3.5 opacity-0 transition group-hover:opacity-100 group-focus-within:opacity-100 " + (expanded ? "rotate-90" : "")} />
        </span>
        <span className="flex min-w-0 flex-1 items-baseline gap-1.5 overflow-hidden">
          <span className="min-w-0 truncate">{group.label}</span>
          {group.repositoryLabel && (
            <span
              className="shrink-0 text-sm font-normal text-ink-faint"
              aria-label={`Repository ${group.repositoryLabel}`}
            >
              {group.repositoryLabel}
            </span>
          )}
        </span>
      </button>
      {canStartSession && (
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            onNewSession();
          }}
          title={newSessionLabel}
          aria-label={newSessionLabel}
          className="sidebar-row-action grid size-7 shrink-0 place-items-center rounded-md text-ink-muted transition hover:bg-bg-sunken hover:text-ink"
        >
          <SquarePen className="size-3.5" />
        </button>
      )}
      {group.kind !== "none" && (
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            onOpenMenu(event.currentTarget);
          }}
          title={`Project actions for ${group.label}`}
          aria-label={`Project actions for ${group.label}`}
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          className={`grid size-7 shrink-0 place-items-center rounded-md text-ink-muted transition hover:bg-bg-sunken hover:text-ink focus-visible:opacity-100 ${
            menuOpen ? "bg-bg-sunken text-ink opacity-100" : "sidebar-row-action"
          }`}
        >
          <MoreHorizontal className="size-3.5" />
        </button>
      )}
    </div>
  );
}
