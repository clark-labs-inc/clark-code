import {
  BufferGeometry,
  CircleGeometry,
  Line,
  LineBasicMaterial,
  Mesh,
  MeshBasicMaterial,
  OrthographicCamera,
  RingGeometry,
  Scene,
  Vector3,
  WebGLRenderer,
} from "three";

import type { RsiLoopRenderState } from "./rsiLoopRenderer";

interface SharedRenderer {
  renderer: WebGLRenderer;
  scene: Scene;
  camera: OrthographicCamera;
  arcs: Line[];
  nodes: Mesh[];
  arrows: Mesh[];
  glow: Mesh;
}

let sharedRenderer: SharedRenderer | null | undefined;

function colorForStage(state: RsiLoopRenderState, index: number): string {
  const status = state.stages[index] ?? "queued";
  if (status === "blocked") return state.colors.warning;
  if (status === "active" || index === state.activeIndex) return state.colors.accent;
  if (status === "complete") return state.colors.complete;
  return state.colors.muted;
}

function createSharedRenderer(): SharedRenderer | null {
  if (typeof document === "undefined") return null;
  try {
    const renderer = new WebGLRenderer({
      alpha: true,
      antialias: true,
      powerPreference: "low-power",
      preserveDrawingBuffer: false,
    });
    renderer.setClearColor(0x000000, 0);

    const scene = new Scene();
    const camera = new OrthographicCamera(-1, 1, 1, -1, 0.1, 10);
    camera.position.z = 2;

    const arcs: Line[] = [];
    const nodes: Mesh[] = [];
    const arrows: Mesh[] = [];
    const stageCount = 5;
    const radius = 0.7;
    const segment = (Math.PI * 2) / stageCount;
    const gap = 0.16;

    for (let index = 0; index < stageCount; index += 1) {
      const start = Math.PI / 2 - index * segment - gap;
      const end = Math.PI / 2 - (index + 1) * segment + gap;
      const points = Array.from({ length: 25 }, (_, pointIndex) => {
        const t = pointIndex / 24;
        const angle = start + (end - start) * t;
        return new Vector3(Math.cos(angle) * radius, Math.sin(angle) * radius, 0);
      });
      const arc = new Line(
        new BufferGeometry().setFromPoints(points),
        new LineBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.9 }),
      );
      scene.add(arc);
      arcs.push(arc);

      const midpoint = start + (end - start) * 0.5;
      const node = new Mesh(
        new CircleGeometry(0.055, 20),
        new MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.96 }),
      );
      node.position.set(Math.cos(midpoint) * radius, Math.sin(midpoint) * radius, 0.02);
      scene.add(node);
      nodes.push(node);

      const arrowAngle = start + (end - start) * 0.72;
      const arrow = new Mesh(
        new BufferGeometry().setFromPoints([
          new Vector3(0, 0.055, 0),
          new Vector3(-0.045, -0.04, 0),
          new Vector3(0.045, -0.04, 0),
        ]).setIndex([0, 1, 2]),
        new MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.88 }),
      );
      arrow.position.set(Math.cos(arrowAngle) * radius, Math.sin(arrowAngle) * radius, 0.03);
      arrow.rotation.z = arrowAngle - Math.PI / 2;
      scene.add(arrow);
      arrows.push(arrow);
    }

    const glow = new Mesh(
      new RingGeometry(0.075, 0.12, 24),
      new MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.2 }),
    );
    scene.add(glow);

    return { renderer, scene, camera, arcs, nodes, arrows, glow };
  } catch {
    return null;
  }
}

/**
 * Render through one shared, off-DOM WebGL context and copy into a normal 2D
 * canvas. Historical timeline widgets therefore do not retain WebGL contexts.
 */
export async function renderRsiLoopWithThree(
  target: HTMLCanvasElement,
  size: number,
  pixelRatio: number,
  state: RsiLoopRenderState,
): Promise<boolean> {
  sharedRenderer ??= createSharedRenderer();
  if (!sharedRenderer || !target.isConnected) return false;

  const drawSize = Math.max(1, Math.round(size * Math.min(pixelRatio, 1.5)));
  sharedRenderer.renderer.setSize(drawSize, drawSize, false);

  sharedRenderer.arcs.forEach((arc, index) => {
    const material = arc.material as LineBasicMaterial;
    material.color.set(colorForStage(state, index));
    material.opacity = state.stages[index] === "queued" ? 0.32 : 0.92;
  });
  sharedRenderer.nodes.forEach((node, index) => {
    const material = node.material as MeshBasicMaterial;
    material.color.set(colorForStage(state, index));
    material.opacity = state.stages[index] === "queued" ? 0.38 : 1;
    const active = index === state.activeIndex;
    node.scale.setScalar(active ? 1.12 + Math.sin(state.phase) * 0.08 : 1);
  });
  sharedRenderer.arrows.forEach((arrow, index) => {
    const material = arrow.material as MeshBasicMaterial;
    material.color.set(colorForStage(state, index));
    material.opacity = state.stages[index] === "queued" ? 0.2 : 0.82;
  });

  const segment = (Math.PI * 2) / 5;
  const activeMidpoint = Math.PI / 2 - state.activeIndex * segment - segment / 2;
  sharedRenderer.glow.position.set(
    Math.cos(activeMidpoint) * 0.7,
    Math.sin(activeMidpoint) * 0.7,
    0.01,
  );
  const glowMaterial = sharedRenderer.glow.material as MeshBasicMaterial;
  glowMaterial.color.set(state.colors.accent);
  glowMaterial.opacity = 0.12 + (Math.sin(state.phase) + 1) * 0.08;
  sharedRenderer.glow.scale.setScalar(1 + (Math.sin(state.phase) + 1) * 0.08);

  sharedRenderer.renderer.render(sharedRenderer.scene, sharedRenderer.camera);
  target.width = drawSize;
  target.height = drawSize;
  const context = target.getContext("2d");
  if (!context) return false;
  context.clearRect(0, 0, drawSize, drawSize);
  context.drawImage(sharedRenderer.renderer.domElement, 0, 0, drawSize, drawSize);
  return true;
}
