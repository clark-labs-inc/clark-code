import { useEffect, useRef, useState } from "react";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { Minus, Plus, RotateCcw, X } from "lucide-react";
import { OVERLAY, accessibleMotion } from "../lib/motion";
import { cn } from "../lib/cn";

const MIN_ZOOM = 1;
const MAX_ZOOM = 5;
const CLICK_ZOOM_STEP = 2;
const BUTTON_ZOOM_STEP = 1.25;
/** Movement past this many pixels turns a press into a pan instead of a click. */
const DRAG_THRESHOLD_PX = 4;

/** Clamp to [MIN_ZOOM, MAX_ZOOM] with two-decimal stability so the percentage
 *  label never reads 249.99999999%. */
export function clampZoom(zoom: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(zoom * 100) / 100));
}

interface LightboxProps {
  src: string;
  alt: string;
  onClose: () => void;
}

/** Fullscreen image viewer: fit by default, click toggles 2x anchored at the
 *  pointer, wheel zooms, dragging pans while zoomed. Escape or a backdrop
 *  click closes. */
export function Lightbox({ src, alt, onClose }: LightboxProps) {
  const reduce = useReducedMotion();
  const [zoom, setZoom] = useState(1);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const imgRef = useRef<HTMLImageElement>(null);
  const drag = useRef<{
    id: number;
    startX: number;
    startY: number;
    baseX: number;
    baseY: number;
    moved: boolean;
  } | null>(null);

  // Take focus so typing while the viewer is open cannot leak into the composer.
  useEffect(() => {
    rootRef.current?.focus();
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      if (event.key === "Escape") onClose();
      else if (event.key === "+" || event.key === "=") setZoom((z) => clampZoom(z * BUTTON_ZOOM_STEP));
      else if (event.key === "-" || event.key === "_") setZoom((z) => clampZoom(z / BUTTON_ZOOM_STEP));
      else if (event.key === "0") reset();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  /** Screen offset of the point being zoomed around, relative to the visual
   *  center of the image (which sits at the viewport center). */
  const anchorAt = (clientX: number, clientY: number): [number, number] => {
    const rect = imgRef.current?.getBoundingClientRect();
    if (!rect || rect.width === 0 || rect.height === 0) return [0, 0];
    return [clientX - (rect.left + rect.width / 2), clientY - (rect.top + rect.height / 2)];
  };

  const zoomAt = (nextRaw: number, ax = 0, ay = 0) => {
    const next = clampZoom(nextRaw);
    const k = next / zoom;
    if (k === 1) return;
    // Keep the anchored content point under the same screen position:
    // t' = k·t + a·(1−k), derived from t + s·p = a before and after.
    setOffset({
      x: k * offset.x + ax * (1 - k),
      y: k * offset.y + ay * (1 - k),
    });
    setZoom(next);
  };

  const reset = () => {
    setOffset({ x: 0, y: 0 });
    setZoom(1);
  };

  // React attaches wheel listeners passively, but zooming must preventDefault
  // to keep the conversation behind the overlay from scrolling.
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const [ax, ay] = anchorAt(event.clientX, event.clientY);
      zoomAt(zoom * Math.exp(-event.deltaY * 0.0015), ax, ay);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  });

  return (
    <m.div
      {...accessibleMotion(OVERLAY, reduce)}
      ref={rootRef}
      tabIndex={-1}
      role="dialog"
      aria-modal="true"
      aria-label={alt ? `Image viewer: ${alt}` : "Image viewer"}
      className="fixed inset-0 z-critical select-none bg-media-stage outline-none"
    >
      {/* Backdrop layer below the stage: clicks anywhere off the image close. */}
      <div aria-hidden onClick={onClose} className="absolute inset-0" />
      <div className="pointer-events-none absolute inset-0 grid place-items-center p-8">
        <img
          ref={imgRef}
          src={src}
          alt={alt}
          draggable={false}
          className={cn(
            "pointer-events-auto max-h-full max-w-full object-contain touch-none",
            dragging ? "cursor-grabbing" : zoom > 1 ? "cursor-grab" : "cursor-zoom-in",
            !dragging && "transition-transform duration-fast ease-agent",
          )}
          style={{ transform: `translate3d(${offset.x}px, ${offset.y}px, 0) scale(${zoom})` }}
          onPointerDown={(event) => {
            event.currentTarget.setPointerCapture(event.pointerId);
            drag.current = {
              id: event.pointerId,
              startX: event.clientX,
              startY: event.clientY,
              baseX: offset.x,
              baseY: offset.y,
              moved: false,
            };
          }}
          onPointerMove={(event) => {
            const d = drag.current;
            // Fit view has nothing to pan — keep it centered until a click zooms.
            if (!d || d.id !== event.pointerId || zoom === MIN_ZOOM) return;
            const dx = event.clientX - d.startX;
            const dy = event.clientY - d.startY;
            if (!d.moved && Math.hypot(dx, dy) <= DRAG_THRESHOLD_PX) return;
            if (!d.moved) setDragging(true);
            d.moved = true;
            setOffset({ x: d.baseX + dx, y: d.baseY + dy });
          }}
          onPointerUp={(event) => {
            const d = drag.current;
            drag.current = null;
            setDragging(false);
            if (event.pointerId !== (d?.id ?? -1)) return;
            if (d!.moved || zoom > 1) {
              // A clean click while zoomed returns to fit; anything else was a pan.
              if (!d!.moved) reset();
              return;
            }
            const [ax, ay] = anchorAt(event.clientX, event.clientY);
            zoomAt(CLICK_ZOOM_STEP, ax, ay);
          }}
        />
      </div>
      <div className="absolute inset-x-0 bottom-5 flex justify-center">
        <div className="flex items-center gap-1 rounded-full border border-border-subtle bg-bg-elevated/90 px-1.5 py-1 shadow-lg backdrop-blur-sm">
          <button
            type="button"
            onClick={() => setZoom((z) => clampZoom(z / BUTTON_ZOOM_STEP))}
            disabled={zoom <= MIN_ZOOM}
            aria-label="Zoom out"
            title="Zoom out"
            className="grid size-7 place-items-center rounded-full text-ink-secondary transition hover:bg-bg-hover hover:text-ink disabled:pointer-events-none disabled:opacity-40"
          >
            <Minus className="size-3.5" />
          </button>
          <span className="w-11 text-center text-xs font-medium tabular-nums text-ink-secondary">
            {Math.round(zoom * 100)}%
          </span>
          <button
            type="button"
            onClick={() => setZoom((z) => clampZoom(z * BUTTON_ZOOM_STEP))}
            disabled={zoom >= MAX_ZOOM}
            aria-label="Zoom in"
            title="Zoom in"
            className="grid size-7 place-items-center rounded-full text-ink-secondary transition hover:bg-bg-hover hover:text-ink disabled:pointer-events-none disabled:opacity-40"
          >
            <Plus className="size-3.5" />
          </button>
          <button
            type="button"
            onClick={reset}
            disabled={zoom === MIN_ZOOM && offset.x === 0 && offset.y === 0}
            aria-label="Fit to screen"
            title="Fit to screen"
            className="grid size-7 place-items-center rounded-full text-ink-secondary transition hover:bg-bg-hover hover:text-ink disabled:pointer-events-none disabled:opacity-40"
          >
            <RotateCcw className="size-3.5" />
          </button>
        </div>
      </div>
      <button
        type="button"
        onClick={onClose}
        aria-label="Close image viewer"
        title="Close"
        className="absolute right-4 top-4 grid size-9 place-items-center rounded-full bg-bg-elevated/90 text-ink-secondary shadow-lg backdrop-blur-sm transition hover:bg-bg-hover hover:text-ink"
      >
        <X className="size-4" />
      </button>
    </m.div>
  );
}

/** An image that opens the fullscreen lightbox when clicked. Drop-in for a
 *  bare <img>: styling travels via `className`. */
export function ZoomableImage({ src, alt, className }: { src: string; alt: string; className?: string }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-label={`View ${alt} full size`}
        title="View full size"
        className="group relative shrink-0 cursor-zoom-in overflow-visible rounded-[inherit]"
      >
        <img src={src} alt={alt} draggable={false} className={className} />
      </button>
      <AnimatePresence>
        {open && <Lightbox src={src} alt={alt} onClose={() => setOpen(false)} />}
      </AnimatePresence>
    </>
  );
}
