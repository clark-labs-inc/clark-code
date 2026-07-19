import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import {
  ListChecks,
  Loader2,
  MessageSquareText,
  Play,
  ShieldAlert,
  ShieldQuestion,
  X,
} from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { wouldAutoApprove } from "../lib/permissions";
import { allowCommand } from "../lib/commandPolicy";
import { cn } from "../lib/cn";
import { Md, MD_CLASSES } from "./Message";
import type { PermissionOption, PermissionOptionKind, PermissionRequest } from "../core-bridge/types";

// One bright action (Allow); everything else stays quiet — Codex restraint.
const OPTION_STYLE: Record<PermissionOptionKind, string> = {
  allow_once: "bg-accent text-on-accent hover:bg-accent-hover",
  allow_always: "bg-bg-tertiary text-ink-secondary hover:bg-bg-hover hover:text-ink",
  reject_once: "text-ink-muted hover:bg-bg-hover hover:text-ink",
  reject_always: "text-danger/80 hover:bg-danger/10 hover:text-danger",
};

/** The permission mode the app switches to after a plan-gate choice. */
function modeAfterPlanChoice(risk: string | undefined, optionId: string) {
  if (risk === "plan" && optionId === "approve_auto") return "auto" as const;
  if (risk === "plan" && optionId === "approve_review") return "ask" as const;
  if (risk === "plan_entry" && optionId === "allow_once") return "plan" as const;
  return null;
}

export function riskTone(risk?: string): { ring: string; chip: string; label: string } | null {
  switch (risk) {
    case "danger":
      return { ring: "bg-danger/10", chip: "bg-danger/15 text-danger", label: "Destructive" };
    case "external":
      return { ring: "bg-info/10", chip: "bg-info/15 text-info", label: "MCP tool" };
    case "billed":
      return { ring: "bg-warning/10", chip: "bg-warning/15 text-warning", label: "Billed image" };
    case "caution":
      return { ring: "bg-warning/10", chip: "bg-warning/15 text-warning", label: "Caution" };
    case "safe":
      return { ring: "bg-bg-secondary", chip: "bg-bg-tertiary text-ink-muted", label: "Safe" };
    case "plan":
    case "plan_entry":
      return { ring: "bg-accent/10", chip: "bg-accent/15 text-accent", label: "Plan" };
    case "confirm":
      // Clark's backend paused before an irreversible action (e.g. sending a
      // message) and wants an explicit go/no-go.
      return {
        ring: "bg-warning/10",
        chip: "bg-warning/15 text-warning",
        label: "Needs confirmation",
      };
    default:
      return null;
  }
}

/** Show the action detail: a unified diff renders red/green; anything else (a
 *  shell command) renders as a plain mono block. */
function DetailView({ text }: { text: string }) {
  if (text.startsWith("diff ")) {
    const lines = text.replace(/^diff .*\n/, "").split("\n");
    return (
      <pre className="mb-2 max-h-56 overflow-auto rounded-md border border-border-subtle bg-bg-sunken px-2.5 py-1.5 font-mono text-xs leading-[1.5]">
        {lines.map((l, i) => {
          const add = l.startsWith("+");
          const del = l.startsWith("-");
          return (
            <div
              key={i}
              className={cn(
                "-mx-1 px-1",
                add && "bg-success/12 text-success",
                del && "bg-danger/12 text-danger",
                !add && !del && "text-ink-muted",
              )}
            >
              {l || " "}
            </div>
          );
        })}
      </pre>
    );
  }
  return (
    <pre className="mb-2 max-h-40 overflow-auto rounded-md border border-border-subtle bg-bg-sunken px-2.5 py-1.5 font-mono text-xs leading-relaxed text-ink-secondary">
      {text}
    </pre>
  );
}

/** Render `backtick` spans in the prompt as quiet mono chips. */
function withChips(text: string) {
  return text.split(/(`[^`]+`)/g).map((part, i) =>
    part.length > 1 && part.startsWith("`") && part.endsWith("`") ? (
      <code
        key={i}
        className="rounded-[5px] bg-chip px-[0.32em] py-[0.12em] font-mono text-[0.85em] text-ink"
      >
        {part.slice(1, -1)}
      </code>
    ) : (
      part
    ),
  );
}

/** Plans are markdown, but the approval surface needs only a quiet count in its
 *  document header. Count top-level ordered-list rows without trying to turn
 *  presentation code into a second markdown parser. */
function topLevelPlanStepCount(markdown: string) {
  return markdown.match(/^\d+\.\s+/gm)?.length ?? 0;
}

/** Inline human-in-the-loop gate — appears in the conversation flow so the user
 *  always sees, in context, exactly what the agent is asking to do (the command
 *  or file, plus a risk classification for shell commands). The motion wrapper is
 *  supplied by the caller's AnimatePresence.
 *
 *  Hidden when the current permission policy auto-grants this request. */
export function PermissionGate({ req }: { req: PermissionRequest }) {
  const resolve = useSessionStore((s) => s.resolvePermission);
  const providePlanFeedback = useSessionStore((s) => s.providePlanFeedback);
  const setPermissionMode = useSessionStore((s) => s.setPermissionMode);
  const mode = useSessionStore((s) => s.permissionMode);
  const project = useSessionStore((s) => s.activeProjectRoot ?? "");
  // Which option was clicked; disables the row until the engine consumes the
  // response (the gate unmounts on the next snapshot) or the send fails.
  const [picked, setPicked] = useState<string | null>(null);
  const [feedbackOpen, setFeedbackOpen] = useState(false);
  const [feedback, setFeedback] = useState("");
  const feedbackRef = useRef<HTMLTextAreaElement>(null);
  // A different request can reuse this mounted gate — reset the pending pick.
  useEffect(() => {
    setPicked(null);
    setFeedbackOpen(false);
    setFeedback("");
  }, [req.id]);
  useEffect(() => {
    if (feedbackOpen) feedbackRef.current?.focus();
  }, [feedbackOpen]);
  if (wouldAutoApprove(mode, req)) return null;

  const tone = riskTone(req.risk);
  const danger = req.risk === "danger";
  const planApproval = req.risk === "plan";
  // A classified shell command (not an MCP/external tool or a file edit).
  const isShellCommand =
    req.risk === "safe" || req.risk === "caution" || req.risk === "danger";

  const onPick = (opt: PermissionOption) => {
    if (picked) return; // a response is already in flight
    // "Always allow this command" persists to the project's allowlist so the
    // engine skips the gate next time (only ever for Safe/Caution commands; MCP
    // "always allow" is handled per-tool by the engine policy instead).
    if (opt.kind === "allow_always" && isShellCommand && req.detail) {
      allowCommand(project, req.detail);
    }
    setPicked(opt.id);
    // On failure the store surfaces the error; re-enable so the user can retry.
    resolve(opt.id)
      .then(() => {
        // A plan-gate choice also picks the app's follow-up mode: approve a
        // plan into "run it for me"/"check each step", or enter plan mode.
        // setPermissionMode syncs the engine, so pill + gate + engine agree.
        const next = modeAfterPlanChoice(req.risk, opt.id);
        if (next) setPermissionMode(next);
      })
      .catch(() => setPicked(null));
  };

  const feedbackOption = req.options.find((opt) => opt.kind === "reject_once");
  const submitFeedback = () => {
    if (!feedbackOption || !feedback.trim() || picked) return;
    setPicked(feedbackOption.id);
    providePlanFeedback(feedback).catch(() => setPicked(null));
  };
  const onFeedbackKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      submitFeedback();
    } else if (event.key === "Escape") {
      event.preventDefault();
      setFeedbackOpen(false);
    }
  };

  const planRunOption = req.options.find((opt) => opt.id === "approve_auto");
  const planReviewOption = req.options.find((opt) => opt.id === "approve_review");

  if (planApproval) {
    const stepCount = req.detail ? topLevelPlanStepCount(req.detail) : 0;
    return (
      <div role="alertdialog" aria-label="Review proposed plan" className="min-w-0">
        <div className="flex items-baseline gap-3 px-1">
          <h2 className="text-lg font-semibold tracking-tight text-ink">Proposed plan</h2>
          {stepCount > 0 && (
            <span className="text-sm tabular-nums text-ink-muted">
              {stepCount} step{stepCount === 1 ? "" : "s"}
            </span>
          )}
        </div>
        <div aria-hidden="true" className="mt-2 flex h-px bg-border">
          <span className="w-24 shrink-0 bg-accent" />
        </div>

        {req.detail && (
          <div
            className={cn(
              MD_CLASSES,
              "max-h-[70vh] overflow-y-auto px-1 py-5 pr-3 text-sm leading-[1.6]",
              "[&_ol]:my-0 [&_ol]:space-y-3 [&_ol]:pl-7 [&_li]:my-0 [&_li]:pl-1",
              "[&_ul]:my-2 [&_ul]:space-y-1 [&_ul]:pl-5",
              "[&_hr]:my-4 [&_hr]:border-0 [&_hr]:border-t [&_hr]:border-border-subtle",
            )}
          >
            <Md>{req.detail}</Md>
          </div>
        )}

        <div className="border-t border-border pt-3">
          <div className="mb-2 flex items-center gap-2 px-1 text-sm font-semibold text-ink">
            <MessageSquareText className="size-4 text-accent" />
            <span>Ready to proceed?</span>
          </div>

          <div className="rounded-lg border border-border bg-bg-elevated p-3">
            {feedbackOpen && feedbackOption && (
              <div className="mb-3">
                <div className="mb-1.5 flex items-center justify-between gap-3">
                  <label
                    htmlFor={`plan-feedback-${req.id}`}
                    className="text-sm font-medium text-ink"
                  >
                    What should change?
                  </label>
                  <button
                    type="button"
                    onClick={() => setFeedbackOpen(false)}
                    disabled={picked !== null}
                    aria-label="Close feedback without changing the plan"
                    title="Close feedback without changing the plan"
                    className="grid size-7 shrink-0 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:opacity-50"
                  >
                    <X className="size-4" />
                  </button>
                </div>
                <textarea
                  ref={feedbackRef}
                  id={`plan-feedback-${req.id}`}
                  value={feedback}
                  onChange={(event) => setFeedback(event.target.value)}
                  onKeyDown={onFeedbackKeyDown}
                  disabled={picked !== null}
                  rows={2}
                  autoCorrect="off"
                  autoCapitalize="off"
                  spellCheck={false}
                  placeholder="Tell Clark what is missing, wrong, or should work differently…"
                  // This field owns its focus cue: one accent border. Suppress
                  // the global outline so it cannot stack into a double ring.
                  style={{ outline: "none" }}
                  className="w-full resize-y rounded-lg border border-border bg-bg px-3 py-2 text-sm leading-relaxed text-ink placeholder:text-ink-faint focus:border-accent disabled:opacity-50"
                />
                <div className="mt-2 flex items-center justify-between gap-3">
                  <span className="text-xs text-ink-faint">
                    Esc to close · ⌘ Enter to send
                  </span>
                  <button
                    type="button"
                    onClick={submitFeedback}
                    disabled={!feedback.trim() || picked !== null}
                    className="relative rounded-lg border border-accent px-3 py-1.5 text-sm font-medium text-accent transition hover:bg-accent/10 disabled:opacity-50"
                  >
                    <span className={cn(picked === feedbackOption.id && "opacity-0")}>
                      Send feedback
                    </span>
                    {picked === feedbackOption.id && (
                      <span className="absolute inset-0 grid place-items-center">
                        <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" />
                      </span>
                    )}
                  </button>
                </div>
              </div>
            )}

            <div className="flex flex-wrap items-stretch gap-2">
              {planRunOption && (
                <button
                  type="button"
                  onClick={() => onPick(planRunOption)}
                  disabled={picked !== null}
                  className="relative flex min-h-12 min-w-[14.25rem] basis-[14.25rem] items-center gap-2.5 rounded-lg bg-accent px-3 py-2 text-left text-on-accent transition hover:bg-accent-hover disabled:opacity-50"
                >
                  <Play className={cn("size-4 shrink-0", picked === planRunOption.id && "opacity-0")} />
                  <span className={cn("min-w-0", picked === planRunOption.id && "opacity-0")}>
                    <span className="block text-sm font-semibold leading-tight">Run the plan</span>
                    <span className="mt-0.5 block text-xs leading-tight text-on-accent/80">
                      Clark works through every step
                    </span>
                  </span>
                  {picked === planRunOption.id && (
                    <span className="absolute inset-0 grid place-items-center">
                      <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" />
                    </span>
                  )}
                </button>
              )}
              {planReviewOption && (
                <button
                  type="button"
                  onClick={() => onPick(planReviewOption)}
                  disabled={picked !== null}
                  className="relative flex min-h-12 min-w-[11.75rem] basis-[11.75rem] items-center gap-2.5 rounded-lg border border-border-strong px-3 py-2 text-left text-ink transition hover:bg-bg-hover disabled:opacity-50"
                >
                  <ListChecks
                    className={cn(
                      "size-4 shrink-0 text-ink-muted",
                      picked === planReviewOption.id && "opacity-0",
                    )}
                  />
                  <span className={cn("min-w-0", picked === planReviewOption.id && "opacity-0")}>
                    <span className="block text-sm font-semibold leading-tight">
                      Review each step
                    </span>
                    <span className="mt-0.5 block text-xs leading-tight text-ink-muted">
                      Ask before changes
                    </span>
                  </span>
                  {picked === planReviewOption.id && (
                    <span className="absolute inset-0 grid place-items-center">
                      <Loader2 className="size-4 animate-[spin_1s_linear_infinite]" />
                    </span>
                  )}
                </button>
              )}
              {feedbackOption && (
                <button
                  type="button"
                  onClick={() => setFeedbackOpen(true)}
                  disabled={picked !== null}
                  className="ml-auto flex min-h-12 items-center gap-2 rounded-lg px-2 py-2 text-sm font-medium text-accent transition hover:bg-accent/10 disabled:opacity-50"
                >
                  <MessageSquareText className="size-4" />
                  Request changes
                </button>
              )}
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div
      role="alertdialog"
      aria-label="Permission required"
      className={cn("rounded-lg p-3.5", tone?.ring ?? "bg-warning/10")}
    >
      <div className="mb-2 flex items-center gap-2 text-sm font-medium text-ink">
        {danger ? (
          <ShieldAlert className="size-4 text-danger" />
        ) : req.risk === "plan" || req.risk === "plan_entry" ? (
          <ListChecks className="size-4 text-accent" />
        ) : (
          <ShieldQuestion className="size-4 text-warning" />
        )}
        <span className="min-w-0 flex-1 truncate">{withChips(req.title)}</span>
        {tone && (
          <span
            className={cn(
              "shrink-0 rounded-md px-1.5 py-0.5 text-xs font-medium",
              tone.chip,
            )}
          >
            {tone.label}
          </span>
        )}
      </div>

      {req.detail && req.risk === "plan_entry" ? (
        // The model's one-line rationale for planning first — prose, not code.
        <p className="mb-3 text-sm leading-relaxed text-ink-secondary">{req.detail}</p>
      ) : (
        req.detail && <DetailView text={req.detail} />
      )}
      {req.risk === "plan_entry" && (
        <p className="mb-3 text-xs text-ink-muted">
          In plan mode Clark researches read-only and proposes a plan — nothing changes until
          you approve it.
        </p>
      )}
      {req.reason && (
        <p className="mb-3 text-xs text-ink-muted">
          Flagged: <span className={cn(danger && "text-danger")}>{req.reason}</span>
        </p>
      )}

      <div className="flex flex-wrap gap-2">
        {req.options
          .map((opt) => (
            <button
              key={opt.id}
              type="button"
              onClick={() => onPick(opt)}
              disabled={picked !== null}
              className={cn(
                "relative rounded-lg px-3 py-1.5 text-sm font-medium transition disabled:opacity-50",
                OPTION_STYLE[opt.kind],
                picked === opt.id && "opacity-100",
              )}
            >
              {/* The spinner overlays the label (which stays, invisible) so the
                  button keeps its exact width when picked — no sibling shove. */}
              <span className={cn(picked === opt.id && "opacity-0")}>{opt.label}</span>
              {picked === opt.id && (
                <span className="absolute inset-0 grid place-items-center">
                  <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" />
                </span>
              )}
            </button>
          ))}
      </div>
    </div>
  );
}
