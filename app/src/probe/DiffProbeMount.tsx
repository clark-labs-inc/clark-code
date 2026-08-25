// Dev-only visual-probe mount (loaded directly through the Vite dev server by
// harness/diff-alignment-probe.mjs; never imported by the app).
import { createRoot } from "react-dom/client";
import { DiffBody, WorkLine } from "../surfaces/work/WorkLine";
import type { ToolCall } from "../core-bridge/types";

const DIFF = [
  "diff --git a/app/src/lib/localAgent.ts b/app/src/lib/localAgent.ts",
  "index 8f2c1aa..b91de04 100644",
  "--- a/app/src/lib/localAgent.ts",
  "+++ b/app/src/lib/localAgent.ts",
  "@@ -108,9 +108,14 @@ export function planTurn(",
  "   const parsed = parseDiff(text);",
  "-  if (!parsed) return null;",
  "+  if (!parsed) {",
  "+    trackSkipped(parsed);",
  "+    return null;",
  "+  }",
  "   const rows = toRows(parsed);",
  "-  return render(rows);",
  "+  return renderRows(rows);",
  "@@ -1236,7 +1241,12 @@ function flushQueue(",
  "   queue.splice(0, 1);",
  "-  drain();",
  "+  if (queue.length > 0) {",
  "+    drain();",
  "+  }",
  "   tick();",
].join("\n");

const editCall: ToolCall = {
  id: "probe-edit",
  title: "Edit app/src/lib/localAgent.ts",
  kind: "edit",
  status: "completed",
  locations: [{ path: "app/src/lib/localAgent.ts", line: 108 }],
  content: [{ type: "text", text: DIFF }],
};
const readCall: ToolCall = {
  id: "probe-read",
  title: "Read app/src/surfaces/work/WorkLine.tsx",
  kind: "read",
  status: "completed",
  locations: [{ path: "app/src/surfaces/work/WorkLine.tsx", line: 77 }],
  content: [{ type: "text", text: "import { memo } from \"react\";\n" }],
};

export function mountProbe(host: HTMLElement) {
  host.style.cssText =
    "padding:24px;background:var(--color-bg-primary);min-height:100vh;font-family:var(--font-sans)";
  host.innerHTML = `
    <h3 style="color:var(--color-ink);font-size:13px;margin:0 0 8px">Real DiffBody</h3>
    <div data-slot="diff" style="max-width:640px;border-radius:8px;overflow:hidden;background:var(--color-bg-elevated)"></div>
    <h3 style="color:var(--color-ink);font-size:13px;margin:20px 0 8px">Real WorkLines</h3>
    <div data-slot="lines" style="max-width:640px;background:var(--color-bg-primary);padding:8px"></div>`;
  const diffSlot = host.querySelector<HTMLElement>("[data-slot='diff']")!;
  const lineSlot = host.querySelector<HTMLElement>("[data-slot='lines']")!;
  createRoot(diffSlot).render(<DiffBody text={DIFF} />);
  createRoot(lineSlot).render(
    <>
      {[editCall, readCall, { ...editCall, id: "probe-edit-2", title: "Edit crates/provider-local/src/prompt.rs" }].map(
        (call) => (
          <div key={call.id} className="mx-2 my-1">
            <WorkLine call={call} active={false} />
          </div>
        ),
      )}
    </>,
  );
}
