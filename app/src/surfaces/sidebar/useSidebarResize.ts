import { useEffect, useRef, useState, type RefObject } from "react";
import { DEFAULT_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, constrainSidebarWidth, loadSidebarWidth, saveSidebarWidth } from "../../lib/sidebarWidth";

export function useSidebarResize(asideRef: RefObject<HTMLElement | null>) {
  // The conversation sidebar is horizontally resizable; the width persists per
  // window and is clamped so the conversation pane keeps a usable minimum.
  const [sidebarWidth, setSidebarWidth] = useState(() => loadSidebarWidth());
  const [viewportWidth, setViewportWidth] = useState(() => window.innerWidth);
  const [resizingSidebar, setResizingSidebar] = useState(false);
  const sidebarResizeCleanupRef = useRef<(() => void) | null>(null);
  const sidebarDoubleClickRef = useRef<{ time: number; x: number } | null>(null);

  const resizeSidebar = (clientX: number) => {
    const left = asideRef.current?.getBoundingClientRect().left ?? 0;
    setSidebarWidth(constrainSidebarWidth(clientX - left, window.innerWidth));
  };
  const finishSidebarResize = () => {
    setResizingSidebar(false);
    setSidebarWidth((current) => {
      const constrained = constrainSidebarWidth(current, window.innerWidth);
      saveSidebarWidth(constrained);
      return constrained;
    });
  };
  const handleSidebarResizeStart = (event: React.MouseEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    // preventDefault on mousedown suppresses the browser's dblclick (Chromium),
    // so detect the double-click here: a repeat press at nearly the same x
    // within the double-click window resets to the default width.
    const now = Date.now();
    const previous = sidebarDoubleClickRef.current;
    sidebarDoubleClickRef.current = { time: now, x: event.clientX };
    if (previous && now - previous.time < 400 && Math.abs(event.clientX - previous.x) <= 4) {
      setSidebarWidth(DEFAULT_SIDEBAR_WIDTH);
      saveSidebarWidth(DEFAULT_SIDEBAR_WIDTH);
      return;
    }
    sidebarResizeCleanupRef.current?.();
    setResizingSidebar(true);
    // Only resize once the pointer actually moves: a plain click (or a second
    // click within the double-click window) must not shift the width.
    let lastX = event.clientX;
    const move = (moveEvent: MouseEvent) => {
      moveEvent.preventDefault();
      if (Math.abs(moveEvent.clientX - lastX) <= 1) return;
      lastX = moveEvent.clientX;
      resizeSidebar(moveEvent.clientX);
    };
    const stop = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", stop);
      sidebarResizeCleanupRef.current = null;
      finishSidebarResize();
    };
    sidebarResizeCleanupRef.current = stop;
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", stop, { once: true });
  };
  const handleSidebarResizeKey = (event: React.KeyboardEvent<HTMLDivElement>) => {
    let next = constrainSidebarWidth(sidebarWidth, window.innerWidth);
    if (event.key === "ArrowRight") next += 24;
    else if (event.key === "ArrowLeft") next -= 24;
    else if (event.key === "Home") next = MIN_SIDEBAR_WIDTH;
    else if (event.key === "End") next = constrainSidebarWidth(window.innerWidth, window.innerWidth);
    else return;
    event.preventDefault();
    const constrained = constrainSidebarWidth(next, window.innerWidth);
    setSidebarWidth(constrained);
    saveSidebarWidth(constrained);
  };

  useEffect(() => {
    const onResize = () =>
      setViewportWidth(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      sidebarResizeCleanupRef.current?.();
    };
  }, []);

  const renderedWidth = constrainSidebarWidth(sidebarWidth, viewportWidth);
  const maxSidebarWidth = constrainSidebarWidth(window.innerWidth, window.innerWidth);
  return { renderedWidth, maxSidebarWidth, resizingSidebar, handleSidebarResizeStart, handleSidebarResizeKey,
    resetSidebarWidth: () => { setSidebarWidth(DEFAULT_SIDEBAR_WIDTH); saveSidebarWidth(DEFAULT_SIDEBAR_WIDTH); } };
}
