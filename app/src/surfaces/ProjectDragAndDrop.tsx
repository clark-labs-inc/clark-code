import { useCallback, useEffect, useState, type ReactNode } from "react";
import { combine } from "@atlaskit/pragmatic-drag-and-drop/combine";
import {
  draggable,
  dropTargetForElements,
} from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import {
  attachClosestEdge,
  extractClosestEdge,
  type Edge,
} from "@atlaskit/pragmatic-drag-and-drop-hitbox/closest-edge";
import { announce } from "@atlaskit/pragmatic-drag-and-drop-live-region";
import { cn } from "../lib/cn";

const PROJECT_DRAG_TYPE = "clark-sidebar-project";
export type ProjectDropEdge = Extract<Edge, "top" | "bottom">;

interface ProjectDragData {
  type: typeof PROJECT_DRAG_TYPE;
  key: string;
  label: string;
}

function projectDragData(data: Record<string | symbol, unknown>): ProjectDragData | null {
  return data.type === PROJECT_DRAG_TYPE
    && typeof data.key === "string"
    && typeof data.label === "string"
    ? { type: PROJECT_DRAG_TYPE, key: data.key, label: data.label }
    : null;
}

export function ProjectDragAndDrop({
  projectKey,
  label,
  enabled,
  onDropProject,
  children,
}: {
  projectKey: string;
  label: string;
  enabled: boolean;
  onDropProject: (sourceKey: string, targetKey: string, edge: ProjectDropEdge) => void;
  children: (dragHandleRef: (element: HTMLElement | null) => void) => ReactNode;
}) {
  const [element, setElement] = useState<HTMLElement | null>(null);
  const [dragHandle, setDragHandle] = useState<HTMLElement | null>(null);
  const [dragging, setDragging] = useState(false);
  const [closestEdge, setClosestEdge] = useState<ProjectDropEdge | null>(null);
  const dragHandleRef = useCallback((node: HTMLElement | null) => setDragHandle(node), []);

  useEffect(() => {
    if (!enabled || !element || !dragHandle) return;
    return combine(
      draggable({
        element,
        dragHandle,
        getInitialData: () => ({ type: PROJECT_DRAG_TYPE, key: projectKey, label }),
        onDragStart: () => {
          setDragging(true);
          announce(`Moving project ${label}. Drop before or after another pinned project.`);
        },
        onDrop: () => setDragging(false),
      }),
      dropTargetForElements({
        element,
        canDrop: ({ source }) => {
          const sourceProject = projectDragData(source.data);
          return sourceProject !== null && sourceProject.key !== projectKey;
        },
        getData: ({ input, element: target }) => attachClosestEdge(
          { type: PROJECT_DRAG_TYPE, key: projectKey, label },
          { input, element: target, allowedEdges: ["top", "bottom"] },
        ),
        onDrag: ({ self }) => {
          const edge = extractClosestEdge(self.data);
          setClosestEdge(edge === "top" || edge === "bottom" ? edge : null);
        },
        onDragLeave: () => setClosestEdge(null),
        onDrop: ({ source, self }) => {
          setClosestEdge(null);
          const sourceProject = projectDragData(source.data);
          const edge = extractClosestEdge(self.data);
          if (!sourceProject || (edge !== "top" && edge !== "bottom")) return;
          onDropProject(sourceProject.key, projectKey, edge);
        },
      }),
    );
  }, [dragHandle, element, enabled, label, onDropProject, projectKey]);

  return (
    <div
      ref={setElement}
      data-sidebar-project={projectKey}
      className={cn("relative", dragging && "opacity-45")}
    >
      {closestEdge && (
        <span
          aria-hidden="true"
          className={cn(
            "pointer-events-none absolute inset-x-1 z-20 h-0.5 rounded-full bg-accent",
            closestEdge === "top" ? "top-0" : "bottom-0",
          )}
        />
      )}
      {children(dragHandleRef)}
    </div>
  );
}
