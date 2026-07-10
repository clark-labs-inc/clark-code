import { useEffect, useState } from "react";
import { ShieldQuestion, ShieldAlert, ListChecks, Loader2 } from "lucide-react";
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

function riskTone(risk?: string): { ring: string; chip: string; label: string } | null {
  switch (risk) {
    case "danger":
      return { ring: "bg-danger/10", chip: "bg-danger/15 text-danger", label: "Destructive" };
    case "external":
      return { ring: "bg-info/10", chip: "bg-info/15 text-info", label: "MCP tool" };
    case "caution":
      return { ring: "bg-warning/10", chip: "bg-warning/15 text-warning", label: "Caution" };
    case "safe":
      return { ring: "bg-bg-secondary", chip: "bg-bg-tertiary text-ink-muted", label: "Safe" };
    case "plan":
      return { ring: "bg-accent/10", chip: "bg-accent/15 text-accent", label: "Plan" };
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

/** Inline human-in-the-loop gate — appears in the conversation flow so the user
 *  always sees, in context, exactly what the agent is asking to do (the command
 *  or file, plus a risk classification for shell commands). The motion wrapper is
 *  supplied by the caller's AnimatePresence.
 *
 *  Hidden when the current permission policy auto-grants this request. */
export function PermissionGate({ req }: { req: PermissionRequest }) {
  const resolve = useSessionStore((s) => s.resolvePermission);
  const mode = useSessionStore((s) => s.permissionMode);
  const project = useSessionStore((s) => s.localSettings.cwd);
  // Which option was clicked; disables the row until the engine consumes the
  // response (the gate unmounts on the next snapshot) or the send fails.
  const [picked, setPicked] = useState<string | null>(null);
  // A different request can reuse this mounted gate — reset the pending pick.
  useEffect(() => setPicked(null), [req.id]);
  if (wouldAutoApprove(mode, req)) return null;

  const tone = riskTone(req.risk);
  const danger = req.risk === "danger";
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
    resolve(opt.id).catch(() => setPicked(null));
  };

  return (
    <div
      role="alertdialog"
      aria-label="Permission required"
      className={cn("rounded-lg p-3.5", tone?.ring ?? "bg-warning/10")}
    >
      <div className="mb-2 flex items-center gap-2 text-sm font-medium text-ink">
        {danger ? (
          <ShieldAlert className="size-4 text-danger" />
        ) : req.risk === "plan" ? (
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

      {req.detail && req.risk === "plan" ? (
        <div
          className={cn(
            MD_CLASSES,
            "mb-3 max-h-72 overflow-auto rounded-md border border-border-subtle bg-bg-sunken px-3 py-2",
          )}
        >
          <Md>{req.detail}</Md>
        </div>
      ) : (
        req.detail && <DetailView text={req.detail} />
      )}
      {req.reason && (
        <p className="mb-3 text-xs text-ink-muted">
          Flagged: <span className={cn(danger && "text-danger")}>{req.reason}</span>
        </p>
      )}

      <div className="flex flex-wrap gap-2">
        {req.options.map((opt) => (
          <button
            key={opt.id}
            onClick={() => onPick(opt)}
            disabled={picked !== null}
            className={cn(
              "rounded-lg px-3 py-1.5 text-sm font-medium transition disabled:opacity-50",
              OPTION_STYLE[opt.kind],
              picked === opt.id && "opacity-100",
            )}
          >
            {picked === opt.id ? (
              <span className="flex items-center gap-1.5">
                <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" />
                {opt.label}
              </span>
            ) : (
              opt.label
            )}
          </button>
        ))}
      </div>
    </div>
  );
}
