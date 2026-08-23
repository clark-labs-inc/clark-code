import { useEffect, useState } from "react";

import type { Activity } from "../../lib/activity";
import type { ToolCall } from "../../core-bridge/types";
import type { SpecLiveStatus } from "../../lib/specProgress";
import { MarkdownContent, MARKDOWN_CLASSES } from "../MarkdownContent";
import { SpecialistFullAccessIndicator } from "../ComposerPermissionPill";
import { SpecRunProgress } from "./SpecRunProgress";
import { SpecLiveDraft } from "./SpecLiveDraft";
import { SpecWorkingState } from "./SpecWorkingState";

const previewCalls: ToolCall[] = [
  {
    id: "read",
    title: "read_file: Reading the existing specification",
    kind: "read",
    status: "completed",
    locations: [{ path: "new_SPEC.md" }],
    content: [],
  },
  {
    id: "stale-search",
    title: "grep: Looking for an older draft",
    kind: "search",
    status: "cancelled",
    locations: [],
    content: [],
  },
  {
    id: "failed-search",
    title: "grep: Searching an unreadable path",
    kind: "search",
    status: "failed",
    locations: [],
    content: [],
  },
  {
    id: "research",
    title: "Finding compatible projects and source documentation",
    kind: "research",
    status: "completed",
    locations: [],
    content: [],
    progress: {
      revision: 6,
      status: "completed",
      latest_activity: "Compared supported repositories",
      phases: [
        { id: "plan", title: "Plan research", status: "completed", steps: [] },
        {
          id: "verify",
          title: "Search and verify sources",
          status: "completed",
          steps: [
            { id: "search", title: "Search official sources", status: "completed" },
            { id: "cross-check", title: "Cross-check product claims", status: "completed" },
          ],
        },
      ],
      agents: [
        {
          id: "docs",
          label: "Project documentation",
          status: "completed",
          summary: "Verified the documented training path",
        },
        {
          id: "product",
          label: "Release notes",
          status: "completed",
          summary: "Confirmed the version range",
        },
      ],
    },
  },
  {
    id: "read-candidate",
    title: "Reading the strongest candidate",
    kind: "fetch",
    status: "completed",
    locations: [],
    content: [],
  },
  {
    id: "acceptance",
    title: "bash: cargo check -p slime-env",
    kind: "execute",
    status: "completed",
    locations: [],
    content: [{ type: "text", text: "Checking slime-env v0.3.1\nFinished in 4.2s\n" }],
  },
  {
    id: "queued-read",
    title: "read_file: Reading the reward interface",
    kind: "read",
    status: "pending",
    locations: [{ path: "src/reward.py" }],
    content: [],
  },
  {
    id: "write",
    title: "Writing the first product-ready draft",
    kind: "edit",
    status: "in_progress",
    locations: [{ path: "slime-supported-grpo_SPEC.md" }],
    content: [],
  },
];

const activity: Activity = {
  busy: true,
  label: "Writing the first product-ready draft",
  progress: 2 / 3,
  steps: { done: 2, total: 3 },
};

const status: SpecLiveStatus = {
  label: "Writing the first product-ready draft",
  detail: "slime-supported-grpo_SPEC.md",
  source: "tool_title",
};

const previewDocument = `# Pre-made Slime environments for GRPO

## Recommendation

Start with a maintained environment that already exposes the reward and rollout interfaces Slime expects. Validate the exact version before committing to it.

## What was verified

- The candidate repository is active and publicly accessible.
- Its documented training path includes a GRPO-compatible workflow.
- Version compatibility still needs a small local acceptance run.

## Safety boundaries

Clark may research, inspect, and update this specification without pausing. It must not delete files or publish changes to GitHub.`;

const DRAFT_SOURCE = `# Pre-made Slime environments for GRPO

## Recommendation

Start with a maintained environment that already exposes the reward and rollout interfaces Slime expects. Validate the exact version before committing to it.

## What was verified

- The candidate repository is active and publicly accessible.
- Its documented training path includes a GRPO-compatible workflow.
- Version compatibility still needs a small local acceptance run.

## Acceptance check

\`\`\`bash
uv run train.py --env slime --algo grpo
- watch for reward curves in wandb
\`\`\`
`;

/** Replays the document the way a provider streams it: whole words, arriving on
 *  a timer, so the harness exercises the real growing-string path. */
function useStreamedDraft(enabled: boolean): string {
  const [chars, setChars] = useState(0);
  useEffect(() => {
    if (!enabled) return;
    const timer = window.setInterval(() => {
      setChars((count) => {
        if (count >= DRAFT_SOURCE.length) return count;
        const next = DRAFT_SOURCE.indexOf(" ", count + 1);
        return next < 0 ? DRAFT_SOURCE.length : next + 1;
      });
    }, 90);
    return () => window.clearInterval(timer);
  }, [enabled]);
  return DRAFT_SOURCE.slice(0, chars);
}

export function SpecRunPreview() {
  const params = new URLSearchParams(window.location.search);
  const showDocument = params.get("step") === "document";
  const showDraft = params.get("step") === "draft";
  const dark = params.has("dark");
  const colorblind = params.has("colorblind");
  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    document.documentElement.classList.toggle("colorblind", colorblind);
    return () => document.documentElement.classList.remove("dark", "colorblind");
  }, [colorblind, dark]);
  const streamed = useStreamedDraft(showDraft);
  return (
    <section className="flex h-screen min-h-0 flex-col overflow-hidden bg-bg text-ink">
      <header className="flex min-h-[5.5rem] shrink-0 items-center gap-3 border-b border-border-subtle px-6 py-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-sm">
            <span className="font-medium text-accent">Spec</span>
            <span className="text-ink-faint">/</span>
            <span className="font-medium">{showDocument ? "Pre-made Slime environments for GRPO" : "New specification"}</span>
          </div>
          <p className="mt-1 text-xs text-ink-faint">{showDocument ? "slime-supported-grpo_SPEC.md" : "new_SPEC.md"} · Working live</p>
        </div>
        <SpecialistFullAccessIndicator specialist="spec" />
      </header>

      <main className="min-h-0 flex-1 overflow-y-auto px-7 pb-10 pt-7">
        {showDraft ? (
          <>
            <SpecRunProgress status={status} activity={activity} calls={previewCalls} compact />
            <SpecLiveDraft
              draft={{ kind: "document", text: streamed, settled: false, callId: "w" }}
            />
          </>
        ) : showDocument ? (
          <>
            <SpecRunProgress status={status} activity={activity} calls={previewCalls} compact />
            <article className={`${MARKDOWN_CLASSES} mx-auto max-w-[44rem] text-sm leading-7 [&_h1]:font-serif [&_h1]:text-4xl [&_h1]:font-semibold [&_h2]:font-serif`}>
              <MarkdownContent>{previewDocument}</MarkdownContent>
            </article>
          </>
        ) : (
          <SpecWorkingState status={status} activity={activity} calls={previewCalls} hasSubmittedPrompt />
        )}
      </main>

      <footer className="shrink-0 border-t border-border-subtle bg-bg px-7 py-4">
        <div className="mx-auto flex min-h-16 max-w-[70rem] items-center rounded-xl border border-border-subtle bg-bg-secondary px-4 text-sm text-ink-faint">
          Add context or steer the spec while Clark works…
        </div>
      </footer>
    </section>
  );
}
