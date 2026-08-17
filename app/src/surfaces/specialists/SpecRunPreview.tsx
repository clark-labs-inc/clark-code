import { useEffect } from "react";

import type { Activity } from "../../lib/activity";
import type { ToolCall } from "../../core-bridge/types";
import { MarkdownContent, MARKDOWN_CLASSES } from "../MarkdownContent";
import { SpecialistFullAccessIndicator } from "../ComposerPermissionPill";
import { SpecRunProgress } from "./SpecRunProgress";
import { SpecWorkingState } from "./SpecWorkingState";

const previewCalls: ToolCall[] = [
  {
    id: "research",
    title: "Finding compatible projects and source documentation",
    kind: "research",
    status: "completed",
    locations: [],
    content: [],
    progress: {
      revision: 2,
      status: "completed",
      latest_activity: "Compared supported repositories",
      phases: [],
      agents: [],
    },
  },
  {
    id: "read",
    title: "Reading the strongest candidate",
    kind: "fetch",
    status: "completed",
    locations: [],
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

const previewDocument = `# Pre-made Slime environments for GRPO

## Recommendation

Start with a maintained environment that already exposes the reward and rollout interfaces Slime expects. Validate the exact version before committing to it.

## What was verified

- The candidate repository is active and publicly accessible.
- Its documented training path includes a GRPO-compatible workflow.
- Version compatibility still needs a small local acceptance run.

## Safety boundaries

Clark may research, inspect, and update this specification without pausing. It must not delete files or publish changes to GitHub.`;

export function SpecRunPreview() {
  const params = new URLSearchParams(window.location.search);
  const showDocument = params.get("step") === "document";
  const dark = params.has("dark");
  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    return () => document.documentElement.classList.remove("dark");
  }, [dark]);
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
        {showDocument ? (
          <>
            <SpecRunProgress activity={activity} calls={previewCalls} compact />
            <article className={`${MARKDOWN_CLASSES} mx-auto max-w-[44rem] text-sm leading-7 [&_h1]:font-serif [&_h1]:text-4xl [&_h1]:font-semibold [&_h2]:font-serif`}>
              <MarkdownContent>{previewDocument}</MarkdownContent>
            </article>
          </>
        ) : (
          <SpecWorkingState activity={activity} calls={previewCalls} hasSubmittedPrompt />
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
