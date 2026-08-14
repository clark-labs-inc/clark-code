import { useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import {
  Check,
  ChevronDown,
  Clock3,
  History,
  Maximize2,
  Monitor,
  Move,
  MousePointerClick,
  Square,
  X,
} from "lucide-react";
import { createPortal } from "react-dom";
import { announce } from "@atlaskit/pragmatic-drag-and-drop-live-region";
import { contentText, imageBlocks, imageSource } from "../../lib/contentBlocks";
import { cn } from "../../lib/cn";
import { useSessionStore } from "../../store/sessionStore";
import type { ToolCall } from "../../core-bridge/types";

interface ComputerFrame {
  id: string;
  source: string;
  appName: string;
  windowTitle: string;
}

type LocalControlState = "live" | "stopped";

interface PanelPoint {
  x: number;
  y: number;
}

interface PanelSize {
  width: number;
  height: number | null;
}

export interface ComputerUseLiveCardProps {
  calls: ToolCall[];
  /** Enables the floating, movable version used by the computer-use prototype. */
  floating?: boolean;
  initialPosition?: PanelPoint;
}

const MIN_PANEL_WIDTH = 300;
const MIN_PANEL_HEIGHT = 340;
const DEFAULT_PANEL_WIDTH = 432;
const PANEL_KEYBOARD_STEP = 16;

function isObservation(call: ToolCall): boolean {
  return call.tool_name === "computer_get_state" || call.tool_name === "computer_observe";
}

export function isComputerUseCall(call: ToolCall): boolean {
  return call.tool_name?.startsWith("computer_") === true;
}

function parseWindow(call: ToolCall): Pick<ComputerFrame, "appName" | "windowTitle"> {
  const line = contentText(call.content)
    .split("\n")
    .find((value) => value.startsWith("Window:"));
  const match = line?.match(/^Window:\s*(.*?)\s+—\s+(.+)$/);
  if (!match) {
    return { appName: "Customer computer", windowTitle: "Current desktop state" };
  }
  return {
    appName: match[1].trim() || "Customer computer",
    windowTitle: match[2].trim().replace(/^"|"$/g, ""),
  };
}

function observationFrames(calls: ToolCall[]): ComputerFrame[] {
  const frames: ComputerFrame[] = [];
  for (const call of calls) {
    if (!isObservation(call)) continue;
    const window = parseWindow(call);
    for (const [index, image] of imageBlocks(call.content).entries()) {
      const source = imageSource(image);
      if (!source) continue;
      const existing = frames.find((frame) => frame.source === source);
      if (existing) {
        Object.assign(existing, { id: `${call.id}:${index}`, ...window });
        continue;
      }
      frames.push({
        id: `${call.id}:${index}`,
        source,
        ...window,
      });
    }
  }
  return frames;
}

function actionLabel(call: ToolCall | undefined): string | null {
  if (!call?.tool_name) return null;
  if (call.status === "in_progress") return "the agent is acting";
  if (call.status === "failed") return "Computer action failed";
  const labels: Record<string, string> = {
    computer_click: "Clicked a control",
    computer_type_text: "Entered text",
    computer_keypress: "Pressed a key",
    computer_scroll: "Scrolled the window",
    computer_drag: "Dragged an item",
    computer_secondary_action: "Opened an action menu",
    computer_select_text: "Selected text",
    computer_set_value: "Updated a value",
    computer_commit_action: "Completed the approved action",
  };
  return labels[call.tool_name] ?? "Updated the computer";
}

function latestAction(calls: ToolCall[]): ToolCall | undefined {
  return [...calls].reverse().find((call) => !isObservation(call) && call.tool_name !== "computer_list_windows");
}

function formatAge(call: ToolCall | undefined): string {
  if (!call) return "Latest observation";
  if (call.status === "in_progress") return "Updating now";
  return "Latest observation";
}

function FrameStack({
  frames,
  selectedIndex,
  onSelect,
}: {
  frames: ComputerFrame[];
  selectedIndex: number;
  onSelect: (index: number) => void;
}) {
  const start = Math.max(0, selectedIndex - 2);
  const visible = frames.slice(start, selectedIndex + 1);
  return (
    <div className="relative aspect-[1.58] min-h-0 overflow-hidden rounded-xl bg-[#08090a] p-3 sm:p-4">
      {visible.slice(0, -1).map((frame, index) => {
        const offset = visible.length - index - 1;
        return (
          <img
            key={frame.id}
            src={frame.source}
            alt={`${frame.appName} previous computer state`}
            className={cn(
              "absolute inset-y-4 left-3 right-3 h-[calc(100%-2rem)] rounded-lg border border-white/10 object-cover object-top opacity-40 shadow-2xl sm:inset-y-5 sm:left-4 sm:right-4",
              offset === 2 && "-translate-x-8 -translate-y-2 rotate-[-3deg]",
              offset === 1 && "-translate-x-4 -translate-y-0.5 rotate-[1.5deg] opacity-60",
            )}
          />
        );
      })}
      {visible.length > 0 && (
        <button
          type="button"
          className="group absolute inset-y-3 left-8 right-3 overflow-hidden rounded-lg border border-white/20 bg-black/20 text-left shadow-2xl transition hover:border-white/40 sm:inset-y-4 sm:left-10 sm:right-4"
          onClick={() => onSelect(selectedIndex)}
          aria-label="Open the current computer screenshot"
        >
          <img
            src={visible[visible.length - 1].source}
            alt={`${visible[visible.length - 1].appName} current computer state`}
            className="h-full w-full object-cover object-top transition duration-slow group-hover:scale-[1.01]"
          />
          <span className="absolute left-2 top-2 inline-flex items-center gap-1 rounded-full border border-white/15 bg-black/65 px-2 py-1 text-xs font-medium uppercase tracking-[0.14em] text-white/80 backdrop-blur-sm">
            <span className="size-1.5 animate-pulse rounded-full bg-emerald-400" />
            Live view
          </span>
        </button>
      )}
    </div>
  );
}

export function ComputerUseLiveCard({
  calls,
  floating = false,
  initialPosition = { x: 0, y: 0 },
}: ComputerUseLiveCardProps) {
  const frames = useMemo(() => observationFrames(calls), [calls]);
  const [selectedIndex, setSelectedIndex] = useState(Math.max(0, frames.length - 1));
  const [historyOpen, setHistoryOpen] = useState(false);
  const [controlState, setControlState] = useState<LocalControlState>("live");
  const [panelPosition, setPanelPosition] = useState<PanelPoint>(initialPosition);
  const [panelSize, setPanelSize] = useState<PanelSize>({ width: DEFAULT_PANEL_WIDTH, height: null });
  const [interaction, setInteraction] = useState<"drag" | "resize" | null>(null);
  const panelRef = useRef<HTMLElement>(null);
  const pointerRef = useRef<{
    start: PanelPoint;
    pointer: PanelPoint;
    size: PanelSize;
  } | null>(null);
  const cancelActive = useSessionStore((state) => state.cancelActive);

  useEffect(() => {
    setSelectedIndex(Math.max(0, frames.length - 1));
  }, [frames.length]);

  useEffect(() => {
    if (!floating || !interaction) return;
    const updateInteraction = (event: PointerEvent) => {
      const panel = panelRef.current;
      const stage = panel?.parentElement;
      const start = pointerRef.current;
      if (!panel || !start || (!floating && !stage)) return;

      const bounds = floating
        ? { width: window.innerWidth, height: window.innerHeight }
        : (() => {
            const rect = stage!.getBoundingClientRect();
            return { width: rect.width, height: rect.height };
          })();
      const panelWidth = interaction === "resize" ? start.size.width : panel.offsetWidth;
      const panelHeight = interaction === "resize" ? start.size.height ?? panel.offsetHeight : panel.offsetHeight;
      const panelTop = floating ? start.start.y : panel.offsetTop;
      const delta = {
        x: event.clientX - start.pointer.x,
        y: event.clientY - start.pointer.y,
      };

      if (interaction === "drag") {
        setPanelPosition({
          x: Math.max(0, Math.min(start.start.x + delta.x, Math.max(0, bounds.width - panelWidth))),
          y: Math.max(0, Math.min(start.start.y + delta.y, Math.max(0, bounds.height - panelHeight))),
        });
        return;
      }

      setPanelSize({
        width: Math.max(MIN_PANEL_WIDTH, Math.min(start.size.width + delta.x, Math.max(MIN_PANEL_WIDTH, bounds.width))),
        height: Math.max(MIN_PANEL_HEIGHT, Math.min((start.size.height ?? panel.offsetHeight) + delta.y, Math.max(MIN_PANEL_HEIGHT, bounds.height - panelTop))),
      });
    };
    const finishInteraction = () => {
      pointerRef.current = null;
      setInteraction(null);
    };
    window.addEventListener("pointermove", updateInteraction);
    window.addEventListener("pointerup", finishInteraction, { once: true });
    window.addEventListener("pointercancel", finishInteraction, { once: true });
    return () => {
      window.removeEventListener("pointermove", updateInteraction);
      window.removeEventListener("pointerup", finishInteraction);
      window.removeEventListener("pointercancel", finishInteraction);
    };
  }, [floating, interaction]);

  useEffect(() => {
    if (!floating) return;
    const keepOnScreen = () => {
      const panel = panelRef.current;
      if (!panel) return;
      setPanelPosition((position) => ({
        x: Math.max(0, Math.min(position.x, Math.max(0, window.innerWidth - panel.offsetWidth))),
        y: Math.max(0, Math.min(position.y, Math.max(0, window.innerHeight - panel.offsetHeight))),
      }));
    };
    keepOnScreen();
    window.addEventListener("resize", keepOnScreen);
    return () => window.removeEventListener("resize", keepOnScreen);
  }, [floating]);

  const beginDrag = (event: ReactPointerEvent<HTMLElement>) => {
    if (!floating || event.button !== 0 || (event.target as HTMLElement).closest("button")) return;
    const panel = panelRef.current;
    if (!panel) return;
    event.preventDefault();
    pointerRef.current = {
      start: panelPosition,
      pointer: { x: event.clientX, y: event.clientY },
      size: panelSize,
    };
    setInteraction("drag");
  };

  const beginResize = (event: ReactPointerEvent<HTMLButtonElement>) => {
    if (!floating || event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    pointerRef.current = {
      start: panelPosition,
      pointer: { x: event.clientX, y: event.clientY },
      size: panelSize,
    };
    setInteraction("resize");
  };

  const movePanelWithKeyboard = (event: React.KeyboardEvent<HTMLElement>) => {
    if (!floating || event.target !== event.currentTarget) return;
    const panel = panelRef.current;
    if (!panel) return;
    const step = event.shiftKey ? PANEL_KEYBOARD_STEP * 3 : PANEL_KEYBOARD_STEP;
    const maxX = Math.max(0, window.innerWidth - panel.offsetWidth);
    const maxY = Math.max(0, window.innerHeight - panel.offsetHeight);
    let next = panelPosition;
    if (event.key === "Home") next = { x: 0, y: 0 };
    else if (event.key === "End") next = { x: maxX, y: maxY };
    else if (event.key === "ArrowLeft") next = { ...panelPosition, x: panelPosition.x - step };
    else if (event.key === "ArrowRight") next = { ...panelPosition, x: panelPosition.x + step };
    else if (event.key === "ArrowUp") next = { ...panelPosition, y: panelPosition.y - step };
    else if (event.key === "ArrowDown") next = { ...panelPosition, y: panelPosition.y + step };
    else return;
    event.preventDefault();
    next = {
      x: Math.max(0, Math.min(next.x, maxX)),
      y: Math.max(0, Math.min(next.y, maxY)),
    };
    setPanelPosition(next);
    announce(`Computer use panel moved to ${Math.round(next.x)} by ${Math.round(next.y)}.`);
  };

  const resizePanelWithKeyboard = (event: React.KeyboardEvent<HTMLButtonElement>) => {
    if (!floating) return;
    const panel = panelRef.current;
    if (!panel) return;
    const step = event.shiftKey ? PANEL_KEYBOARD_STEP * 3 : PANEL_KEYBOARD_STEP;
    const currentHeight = panelSize.height ?? panel.offsetHeight;
    let next = { width: panelSize.width, height: currentHeight };
    if (event.key === "Home") {
      event.preventDefault();
      event.stopPropagation();
      setPanelSize({ width: DEFAULT_PANEL_WIDTH, height: null });
      announce("Computer use panel size reset.");
      return;
    } else if (event.key === "ArrowLeft") next.width -= step;
    else if (event.key === "ArrowRight") next.width += step;
    else if (event.key === "ArrowUp") next.height -= step;
    else if (event.key === "ArrowDown") next.height += step;
    else return;
    event.preventDefault();
    event.stopPropagation();
    next = {
      width: Math.max(MIN_PANEL_WIDTH, Math.min(next.width, window.innerWidth - panelPosition.x)),
      height: Math.max(MIN_PANEL_HEIGHT, Math.min(next.height, window.innerHeight - panelPosition.y)),
    };
    setPanelSize(next);
    announce(`Computer use panel resized to ${Math.round(next.width)} by ${Math.round(next.height)}.`);
  };

  if (frames.length === 0) return null;

  const current = frames[selectedIndex] ?? frames[frames.length - 1];
  const action = latestAction(calls);
  const actionText = actionLabel(action);
  const active = calls.some((call) => call.status === "in_progress") && controlState === "live";
  const controlText = controlState === "stopped"
    ? "the agent stopped"
    : active
      ? "the agent is working"
      : "Computer state captured";

  const stop = async () => {
    setControlState("stopped");
    await cancelActive();
  };

  const card = (
    <section
      ref={panelRef}
      aria-label="the agent computer use"
      style={floating ? {
        width: `${panelSize.width}px`,
        height: panelSize.height ? `${panelSize.height}px` : undefined,
        left: `${panelPosition.x}px`,
        top: `${panelPosition.y}px`,
        maxWidth: floating ? "calc(100vw - 1.5rem)" : undefined,
      } : undefined}
      className={cn(
        "ml-auto w-full max-w-[27rem] overflow-hidden rounded-2xl border border-white/[0.08] bg-[#171819] text-white shadow-[0_16px_45px_rgba(0,0,0,0.22)]",
        floating && "fixed z-50 max-w-none",
        interaction === "drag" && "cursor-grabbing",
      )}
    >
      <header
        onPointerDown={beginDrag}
        onKeyDown={movePanelWithKeyboard}
        tabIndex={floating ? 0 : undefined}
        aria-label={floating ? "Move computer use panel with arrow keys. Home moves to the top left and End moves to the bottom right." : undefined}
        title={floating ? "Drag to move · Arrow keys move · Home/End move to corners" : undefined}
        className={cn(
          "flex items-center gap-3 px-3.5 py-3 sm:px-4",
          floating && "cursor-grab select-none",
        )}
      >
        <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-white/[0.08] text-white/80">
          <Monitor className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-sm font-medium">
            <span>Computer use</span>
            <span className={cn(
              "size-1.5 rounded-full",
              active ? "animate-pulse bg-emerald-400" : controlState === "stopped" ? "bg-red-400" : "bg-white/35",
            )} />
          </div>
          <div className="truncate text-xs text-white/50">
            {current.appName}{current.windowTitle ? ` · ${current.windowTitle}` : ""}
          </div>
        </div>
        {floating && <Move aria-hidden="true" className="size-3.5 shrink-0 text-white/25" />}
        <button
          type="button"
          onClick={() => setHistoryOpen((open) => !open)}
          aria-expanded={historyOpen}
          aria-label={historyOpen ? "Hide computer screenshot history" : "Show computer screenshot history"}
          className="grid size-8 place-items-center rounded-lg text-white/50 transition hover:bg-white/[0.08] hover:text-white"
        >
          <ChevronDown className={cn("size-4 transition-transform", historyOpen && "rotate-180")} />
        </button>
      </header>

      <div className="px-3 pb-3 sm:px-4 sm:pb-4">
        <FrameStack frames={frames} selectedIndex={selectedIndex} onSelect={setSelectedIndex} />
      </div>

      {historyOpen && frames.length > 1 && (
        <div className="border-t border-white/[0.08] px-3 py-3 sm:px-4">
          <div className="mb-2 flex items-center gap-1.5 text-xs font-medium uppercase tracking-[0.14em] text-white/40">
            <History className="size-3" />
            Recent observations
          </div>
          <div className="flex gap-2 overflow-x-auto pb-0.5">
            {frames.map((frame, index) => (
              <button
                key={frame.id}
                type="button"
                onClick={() => setSelectedIndex(index)}
                aria-label={`View observation ${index + 1}`}
                className={cn(
                  "size-14 shrink-0 overflow-hidden rounded-md border bg-black/30 transition",
                  index === selectedIndex ? "border-white/70 ring-1 ring-white/20" : "border-white/10 opacity-60 hover:opacity-100",
                )}
              >
                <img src={frame.source} alt="" className="h-full w-full object-cover object-top" />
              </button>
            ))}
          </div>
        </div>
      )}

      <div className="flex items-center gap-2 border-t border-white/[0.08] px-3.5 py-3 sm:px-4">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5 text-xs font-medium text-white/80">
            {controlState === "stopped" ? <X className="size-3.5 text-red-300" /> : <MousePointerClick className="size-3.5 text-white/50" />}
            <span className="truncate">{controlState === "live" ? actionText ?? controlText : controlText}</span>
          </div>
          <div className="mt-0.5 flex items-center gap-1 text-xs text-white/40">
            <Clock3 className="size-3" />
            {formatAge(action)}
          </div>
        </div>
        {controlState === "live" ? (
            <button
              type="button"
              onClick={() => void stop()}
              aria-label="Stop the agent computer use"
              className="grid size-8 place-items-center rounded-lg border border-red-300/25 text-red-200 transition hover:bg-red-300/10"
            >
              <Square className="size-3.5 fill-current" />
            </button>
        ) : (
          <span className="inline-flex items-center gap-1.5 rounded-lg border border-white/10 px-2.5 py-1.5 text-xs text-white/45">
            <Check className="size-3.5" />
            Stopped
          </span>
        )}
      </div>
      {floating && (
        <button
          type="button"
          onPointerDown={beginResize}
          onKeyDown={resizePanelWithKeyboard}
          aria-label="Resize computer use panel with arrow keys. Home resets the size."
          title="Drag to resize · Arrow keys resize · Home resets"
          className={cn(
            "absolute bottom-1 right-1 grid size-5 cursor-nwse-resize place-items-center rounded-md text-white/35 transition hover:bg-white/[0.08] hover:text-white/75",
            interaction === "resize" && "bg-white/[0.08] text-white/75",
          )}
        >
          <Maximize2 className="size-3" />
        </button>
      )}
    </section>
  );

  return floating && typeof document !== "undefined" ? createPortal(card, document.body) : card;
}
