import { useEffect, useState } from "react";
import { ListChecks, Loader2, ShieldAlert, ShieldQuestion } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { wouldAutoApprove } from "../lib/permissions";
import { allowCommand } from "../lib/commandPolicy";
import { cn } from "../lib/cn";
import type { PermissionOption, PermissionOptionKind, PermissionRequest } from "../core-bridge/types";

const OPTION_STYLE: Record<PermissionOptionKind, string> = {
  allow_once: "bg-accent text-on-accent hover:bg-accent-hover",
  allow_always: "bg-bg-tertiary text-ink-secondary hover:bg-bg-hover hover:text-ink",
  reject_once: "text-ink-muted hover:bg-bg-hover hover:text-ink",
  reject_always: "text-danger/80 hover:bg-danger/10 hover:text-danger",
};

export function riskTone(risk?: string): { ring: string; chip: string; label: string } | null {
  switch (risk) {
    case "danger": return { ring: "bg-danger/10", chip: "bg-danger/15 text-danger", label: "Destructive" };
    case "network": return { ring: "bg-info/10", chip: "bg-info/15 text-info", label: "Network access" };
    case "sandbox": return { ring: "bg-warning/10", chip: "bg-warning/15 text-warning", label: "Outside sandbox" };
    case "external": return { ring: "bg-info/10", chip: "bg-info/15 text-info", label: "External access" };
    case "billed": return { ring: "bg-warning/10", chip: "bg-warning/15 text-warning", label: "Billed image" };
    case "caution": return { ring: "bg-warning/10", chip: "bg-warning/15 text-warning", label: "Caution" };
    case "safe": return { ring: "bg-bg-secondary", chip: "bg-bg-tertiary text-ink-muted", label: "Safe" };
    case "plan_entry": return { ring: "bg-accent/10", chip: "bg-accent/15 text-accent", label: "Plan Mode" };
    case "confirm": return { ring: "bg-warning/10", chip: "bg-warning/15 text-warning", label: "Needs confirmation" };
    default: return null;
  }
}

function DetailView({ text }: { text: string }) {
  if (!text.startsWith("diff ")) {
    return <pre className="mb-2 max-h-40 overflow-auto rounded-md border border-border-subtle bg-bg-sunken px-2.5 py-1.5 font-mono text-xs leading-relaxed text-ink-secondary">{text}</pre>;
  }
  return (
    <pre className="mb-2 max-h-56 overflow-auto rounded-md border border-border-subtle bg-bg-sunken px-2.5 py-1.5 font-mono text-xs leading-[1.5]">
      {text.replace(/^diff .*\n/, "").split("\n").map((line, index) => (
        <div key={index} className={cn("-mx-1 px-1", line.startsWith("+") && "bg-success/12 text-success", line.startsWith("-") && "bg-danger/12 text-danger", !line.startsWith("+") && !line.startsWith("-") && "text-ink-muted")}>{line || " "}</div>
      ))}
    </pre>
  );
}

function withChips(text: string) {
  return text.split(/(`[^`]+`)/g).map((part, index) =>
    part.length > 1 && part.startsWith("`") && part.endsWith("`")
      ? <code key={index} className="rounded-[5px] bg-chip px-[0.32em] py-[0.12em] font-mono text-[0.85em] text-ink">{part.slice(1, -1)}</code>
      : part,
  );
}

/** Action approval only. Proposed-plan decisions have their own typed card. */
export function PermissionGate({ req }: { req: PermissionRequest }) {
  const resolve = useSessionStore((state) => state.resolvePermission);
  const setCollaborationMode = useSessionStore((state) => state.setCollaborationMode);
  const policy = useSessionStore((state) => state.approvalPolicy);
  const project = useSessionStore((state) => state.activeProjectRoot ?? "");
  const [picked, setPicked] = useState<string | null>(null);
  useEffect(() => setPicked(null), [req.id]);
  if (wouldAutoApprove(policy, req)) return null;

  const tone = riskTone(req.risk);
  const danger = req.risk === "danger";
  const boundary = req.risk === "network" || req.risk === "sandbox";
  const isShell = req.risk === "safe" || req.risk === "caution" || req.risk === "danger" || boundary;
  const onPick = (option: PermissionOption) => {
    if (picked) return;
    if (option.kind === "allow_always" && isShell && req.detail) allowCommand(project, req.detail);
    setPicked(option.id);
    resolve(option.id)
      .then(() => {
        if (req.risk === "plan_entry" && option.kind.startsWith("allow")) {
          setCollaborationMode("plan");
        }
      })
      .catch(() => setPicked(null));
  };

  return (
    <div role="alertdialog" aria-label="Permission required" className={cn("rounded-lg p-3.5", tone?.ring ?? "bg-warning/10")}>
      <div className="mb-2 flex items-center gap-2 text-sm font-medium text-ink">
        {danger ? <ShieldAlert className="size-4 text-danger" /> : req.risk === "plan_entry" ? <ListChecks className="size-4 text-accent" /> : <ShieldQuestion className="size-4 text-warning" />}
        <span className="min-w-0 flex-1 truncate">{withChips(req.title)}</span>
        {tone && <span className={cn("shrink-0 rounded-md px-1.5 py-0.5 text-xs font-medium", tone.chip)}>{tone.label}</span>}
      </div>
      {req.detail && req.risk === "plan_entry" ? <p className="mb-3 text-sm leading-relaxed text-ink-secondary">{req.detail}</p> : req.detail ? <DetailView text={req.detail} /> : null}
      {req.risk === "plan_entry" && <p className="mb-3 text-xs text-ink-muted">Clark will research read-only and return a decision-complete plan. Execution begins only after you approve it.</p>}
      {boundary && <p className="mb-3 text-xs text-ink-muted">Approval applies only to this command. “Always allow” remembers this exact command, not general host access.</p>}
      {req.reason && <p className="mb-3 text-xs text-ink-muted">{boundary ? "Why" : "Flagged"}: <span className={cn(danger && "text-danger")}>{req.reason}</span></p>}
      <div className="flex flex-wrap gap-2">
        {req.options.map((option) => (
          <button key={option.id} type="button" onClick={() => onPick(option)} disabled={picked !== null} className={cn("relative rounded-lg px-3 py-1.5 text-sm font-medium transition disabled:opacity-50", OPTION_STYLE[option.kind])}>
            <span className={cn(picked === option.id && "opacity-0")}>{option.label}</span>
            {picked === option.id && <span className="absolute inset-0 grid place-items-center"><Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" /></span>}
          </button>
        ))}
      </div>
    </div>
  );
}
