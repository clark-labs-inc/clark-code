import { useEffect, useState } from "react";
import { AlertTriangle, Loader2, ShieldCheck } from "lucide-react";
import {
  getBridge,
  type CoreBridge,
  type LocalSandboxStatus,
} from "../core-bridge/bridge";
import { cn } from "../lib/cn";
import { Card, GroupLabel, Row } from "./settings/Primitives";

interface SandboxSetupCardProps {
  cwd: string;
  compact?: boolean;
  onStatusChange?: (status: LocalSandboxStatus | null) => void;
}

export interface LocalSandboxObservation {
  cwd: string;
  status: LocalSandboxStatus | null;
}

export function sandboxStatusForCwd(
  observation: LocalSandboxObservation | null,
  cwd: string,
): LocalSandboxStatus | null {
  return observation?.cwd === cwd ? observation.status : null;
}

export function sandboxBlocksSubmission(
  required: boolean,
  status: LocalSandboxStatus | null,
): boolean {
  return required && status?.state !== "enforced";
}

export function sandboxGateRequired({
  localTarget,
  remoteTarget,
  fullAccess,
  cwd,
  nativeHost,
  statusSupported,
}: {
  localTarget: boolean;
  remoteTarget: boolean;
  fullAccess: boolean;
  cwd: string;
  nativeHost: boolean;
  statusSupported: boolean;
}): boolean {
  return (
    localTarget
    && !remoteTarget
    && !fullAccess
    && cwd.trim().length > 0
    && (nativeHost || statusSupported)
  );
}

export async function readLocalSandboxStatus(
  bridge: CoreBridge,
  cwd: string,
): Promise<LocalSandboxStatus> {
  if (!bridge.localSandboxStatus) {
    throw new Error("This Clark build cannot inspect the local command sandbox");
  }
  return bridge.localSandboxStatus(cwd);
}

export function SandboxSetupCard({
  cwd,
  compact = false,
  onStatusChange,
}: SandboxSetupCardProps) {
  const [status, setStatus] = useState<LocalSandboxStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    let cancelled = false;
    setStatus(null);
    onStatusChange?.(null);
    setError("");
    if (!cwd.trim()) return () => { cancelled = true; };
    void getBridge()
      .then((bridge) => readLocalSandboxStatus(bridge, cwd))
      .then((next) => {
        if (!cancelled) {
          setStatus(next);
          onStatusChange?.(next);
        }
      })
      .catch((reason) => {
        if (!cancelled) setError(String(reason));
      });
    return () => { cancelled = true; };
  }, [cwd, onStatusChange]);

  if ((!status && !error) || status?.state === "enforced") return null;

  const canSetup = status?.state === "setup_required" && status.setup_available;
  const runSetup = () => {
    setBusy(true);
    setError("");
    void getBridge()
      .then((bridge) => {
        if (!bridge.setupLocalSandbox) throw new Error("Sandbox setup is unavailable");
        return bridge.setupLocalSandbox(cwd);
      })
      .then((next) => {
        setStatus(next);
        onStatusChange?.(next);
      })
      .catch((reason) => setError(String(reason)))
      .finally(() => setBusy(false));
  };
  const setupButton = canSetup && (
    <button
      type="button"
      disabled={busy}
      onClick={runSetup}
      className={cn(
        "flex shrink-0 items-center gap-1.5 rounded-lg bg-accent px-2.5 py-1.5 text-xs font-medium text-white transition hover:brightness-110 disabled:opacity-60",
        compact && "self-start",
      )}
    >
      {busy && <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" />}
      {busy ? "Waiting for Windows…" : "Enable sandbox"}
    </button>
  );

  if (compact) {
    return (
      <div
        role="status"
        aria-live="polite"
        className="composer-column-width mx-auto mb-2 flex w-full items-start gap-3 rounded-xl border border-warning/25 bg-warning/10 px-3.5 py-3"
      >
        {canSetup
          ? <ShieldCheck className="mt-0.5 size-4 shrink-0 text-accent" />
          : <AlertTriangle className="mt-0.5 size-4 shrink-0 text-warning" />}
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-ink">
            {canSetup ? "Enable the Windows command sandbox" : "Command sandbox unavailable"}
          </p>
          <p className="mt-0.5 text-xs leading-relaxed text-ink-muted">
            {canSetup
              ? "Clark needs one Windows approval before it can run project commands safely. Setup is reused after restart."
              : (status?.reason || error
                || "Repair the Clark Code installation before running local commands.")}
          </p>
          {error && <p className="mt-1.5 text-xs text-danger">{error}</p>}
        </div>
        {setupButton}
      </div>
    );
  }

  return (
    <div>
      <GroupLabel>Local command sandbox</GroupLabel>
      <Card>
        <Row
          icon={canSetup
            ? <ShieldCheck className="size-4 text-accent" />
            : <AlertTriangle className="size-4 text-warning" />}
          name={canSetup ? "Enable the Windows sandbox" : "Sandbox unavailable"}
          sub={canSetup
            ? "One Windows approval creates Clark’s offline identity and network block. After that, Clark enrolls folders you own without prompting; protected folders ask only when Windows requires it. Clark Cloud stays available through its brokered host tool."
            : (status?.reason || error
              || "This platform sandbox is not available on this machine.")}
        >
          {setupButton}
        </Row>
        {error && <p className="px-3.5 py-2.5 text-xs text-danger">{error}</p>}
      </Card>
    </div>
  );
}
