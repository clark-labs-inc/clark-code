import { memo } from "react";
import { WorkLine } from "./WorkLine";
import { sameContentBlocks } from "../../lib/contentBlocks";
import { summarizeEdits } from "../../lib/diff";
import type { ToolCall } from "../../core-bridge/types";

/** A contiguous run of agent tool calls + a git-style edit summary. Memoized so
 *  the (often expensive) diff summarization isn't recomputed on every streamed
 *  token of a *later* message — only when these calls themselves change. */
function WorkBlockImpl({ calls }: { calls: ToolCall[] }) {
  const edits = summarizeEdits(calls);
  return (
    <div
      // Containment applied unconditionally (not toggled on settle): the `auto`
      // intrinsic-size keyword stores the real height after first paint, so
      // there's no recalc/jump when a block finishes — and no scrollback jump.
      className="-my-1 flex flex-col [content-visibility:auto] [contain-intrinsic-size:auto_3rem]"
    >
      {calls.map((call) => (
        <WorkLine key={call.id} call={call} active={call.status === "in_progress"} />
      ))}
      {edits && (
        <div className="mt-1 flex items-center gap-2 pl-[1.4rem] text-xs text-ink-faint">
          <span>
            {edits.files} file{edits.files === 1 ? "" : "s"} changed
          </span>
          <span className="font-mono tabular-nums">
            {edits.adds > 0 && <span className="text-success">+{edits.adds}</span>}
            {edits.adds > 0 && edits.dels > 0 && " "}
            {edits.dels > 0 && <span className="text-danger">−{edits.dels}</span>}
          </span>
        </div>
      )}
    </div>
  );
}

function sameCalls(a: ToolCall[], b: ToolCall[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i];
    const y = b[i];
    if (
      x.id !== y.id ||
      x.status !== y.status ||
      x.title !== y.title ||
      !sameContentBlocks(x.content, y.content) ||
      x.locations.length !== y.locations.length
    ) {
      return false;
    }
  }
  return true;
}

export const WorkBlock = memo(WorkBlockImpl, (a, b) => sameCalls(a.calls, b.calls));
