import { useLayoutEffect, useRef, type ReactNode } from "react";
import { useReducedMotion } from "motion/react";
import { workspaceNavigationMotion } from "../lib/motion";

/** Keep the shell mounted while preserving each child's existing lifecycle.
 * Content is never hidden or held behind an exit animation. In particular,
 * streaming updates cannot restart this cue or remount the composer. */
export function WorkspaceStage({
  stage,
  navigationKey,
  children,
}: {
  stage: string;
  navigationKey: string;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const previous = useRef(navigationKey);
  const reduce = useReducedMotion();
  useLayoutEffect(() => {
    if (previous.current === navigationKey) return;
    previous.current = navigationKey;
    const element = ref.current;
    if (!element?.animate) return;
    // Start almost fully opaque, avoiding the blank full-pane flash of an
    // opacity-zero fade. Reduced motion removes the small directional cue.
    const { frames, options } = workspaceNavigationMotion(reduce);
    const animation = element.animate(frames, options);
    animation.id = "workspace-navigation";
    return () => animation.cancel();
  }, [navigationKey, reduce]);

  return (
    <div ref={ref} data-workspace-stage={stage} className="flex min-h-0 flex-1 flex-col">
      {children}
    </div>
  );
}
