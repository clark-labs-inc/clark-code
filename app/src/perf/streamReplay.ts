/** Drives the store with a synthetic streaming run at a fixed cadence.
 *
 *  Scenario A1: measure the frontend in isolation, inside the real WebView, at
 *  a rate we control and with no model spend. The transcript grows turn by turn
 *  over the run, because the cost under investigation scales with transcript
 *  length — a fixed-size replay would measure the one case that does not hurt.
 *
 *  This deliberately does NOT exercise the host emit path. For that, point the
 *  app at the local SSE fixture (scenario A2), which streams through the real
 *  provider, batching, and IPC. Both tiers are needed; neither substitutes for
 *  the other, and the report must say which one produced a number. */

interface StoreLike {
  getState: () => { snapshot?: unknown; session?: unknown };
  setState: (partial: Record<string, unknown>) => void;
}

/** The session the transcript belongs to.
 *
 *  Without this the conversation surface never mounts (`Conversation` returns
 *  null when `session` is absent) and the run would measure an idle welcome
 *  screen while reporting itself as a streaming benchmark. */
const REPLAY_SESSION = {
  id: "perf-replay",
  provider: "local",
  collaboration_mode: "default",
  capabilities: {
    streaming: true,
    permissions: true,
    fs: true,
    terminal: true,
    load_session: true,
    modes: ["default"],
    collaboration_modes: ["default"],
  },
} as const;

/** One completed turn. Prose plus fenced code, because a fenced block is the
 *  most expensive thing the transcript renders. */
function turn(index: number, codeLines: number): Array<Record<string, unknown>> {
  const runId = `perf-${index}`;
  const code = Array.from(
    { length: codeLines },
    (_, line) => `  const value${line} = compute(${line}, ${index}); // step ${line}`,
  ).join("\n");
  const body = [
    `## Step ${index}`,
    "",
    "Walking through the change:",
    "",
    "```typescript",
    `export function handler${index}() {`,
    code,
    "}",
    "```",
    "",
    "| Path | Warm | Cold |",
    "| --- | --- | --- |",
    "| Reattach | 19ms | n/a |",
    "",
    "Closing prose for this turn. ".repeat(6),
  ].join("\n");
  return [
    { item: "message", run: runId, role: "user", blocks: [{ type: "text", text: `turn ${index}` }] },
    {
      item: "message",
      run: runId,
      role: "agent",
      phase: "final_answer",
      blocks: [{ type: "text", text: body }],
    },
  ];
}

export interface ReplayOptions {
  /** Completed turns to build up over the run. */
  turns?: number;
  /** Milliseconds between snapshot pushes. 16 approximates the host's ceiling. */
  cadenceMs?: number;
  /** Lines of code in each turn's fenced block. */
  codeLines?: number;
  /** Token-sized pushes per turn, so the tail message grows character by
   *  character the way a real stream does. */
  chunksPerTurn?: number;
}

/** Push snapshots into the store at a fixed cadence until the run completes. */
export async function replayStream(
  store: StoreLike,
  options: ReplayOptions = {},
): Promise<{ pushes: number; turns: number; elapsedMs: number }> {
  const { turns: turnCount = 40, cadenceMs = 16, codeLines = 60, chunksPerTurn = 40 } = options;
  const started = performance.now();
  store.setState({ session: { ...REPLAY_SESSION }, connecting: false, opening: null });
  const timeline: Array<Record<string, unknown>> = [];
  const runs: Record<string, unknown> = {};
  let pushes = 0;

  const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

  for (let index = 0; index < turnCount; index += 1) {
    const [userMessage, agentMessage] = turn(index, codeLines);
    const runId = `perf-${index}`;
    runs[runId] = { id: runId, status: "running", checkpoint: "perf" };
    timeline.push(userMessage);
    const full = ((agentMessage.blocks as Array<{ text: string }>)[0]).text;
    const streaming: Record<string, unknown> = {
      ...agentMessage,
      blocks: [{ type: "text", text: "" }],
    };
    timeline.push(streaming);

    // Grow the tail message in chunks, re-pushing the whole snapshot each time
    // — the same shape the host produces today.
    for (let chunk = 1; chunk <= chunksPerTurn; chunk += 1) {
      const upTo = Math.ceil((full.length * chunk) / chunksPerTurn);
      streaming.blocks = [{ type: "text", text: full.slice(0, upTo) }];
      store.setState({
        snapshot: {
          ...(store.getState().snapshot as object),
          session: REPLAY_SESSION.id,
          timeline: [...timeline],
          runs: { ...runs },
        },
      });
      pushes += 1;
      await sleep(cadenceMs);
    }
    runs[runId] = { id: runId, status: "done", outcome: { status: "done" }, checkpoint: "perf" };
  }

  // Push the settled state, or the last turn stays "running" and the activity
  // animation keeps working after the replay is over.
  store.setState({
    snapshot: {
      ...(store.getState().snapshot as object),
      session: REPLAY_SESSION.id,
      timeline: [...timeline],
      runs: { ...runs },
    },
  });
  pushes += 1;

  return { pushes, turns: turnCount, elapsedMs: performance.now() - started };
}
