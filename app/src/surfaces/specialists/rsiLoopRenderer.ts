export type RsiLoopVisualStatus = "complete" | "active" | "queued" | "blocked";

export interface RsiLoopRenderState {
  stages: readonly RsiLoopVisualStatus[];
  activeIndex: number;
  phase: number;
  colors: {
    accent: string;
    complete: string;
    warning: string;
    muted: string;
  };
}

type RenderLoop = (
  target: HTMLCanvasElement,
  size: number,
  pixelRatio: number,
  state: RsiLoopRenderState,
) => Promise<boolean>;

let rendererPromise: Promise<RenderLoop> | null = null;

/**
 * Three.js is fetched only when a visible RSI loop first needs WebGL. The
 * runtime imports named primitives so the build can discard the rest of Three.
 */
export async function renderRsiLoop(
  target: HTMLCanvasElement,
  size: number,
  pixelRatio: number,
  state: RsiLoopRenderState,
): Promise<boolean> {
  rendererPromise ??= import("./rsiLoopThreeRuntime")
    .then((module) => module.renderRsiLoopWithThree);
  const render = await rendererPromise;
  return render(target, size, pixelRatio, state);
}
