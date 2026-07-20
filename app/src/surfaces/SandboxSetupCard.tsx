import { useEffect, useState } from "react";
import { AlertTriangle, Loader2, ShieldCheck } from "lucide-react";
import { getBridge, type LocalSandboxStatus } from "../core-bridge/bridge";
import { Card, GroupLabel, Row } from "./settings/Primitives";

export function SandboxSetupCard({ cwd }: { cwd: string }) {
  const [status, setStatus] = useState<LocalSandboxStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    let cancelled = false;
    setStatus(null);
    setError("");
    if (!cwd.trim()) return () => { cancelled = true; };
    void getBridge()
      .then((bridge) => bridge.localSandboxStatus?.(cwd))
      .then((next) => {
        if (!cancelled && next) setStatus(next);
      })
      .catch((reason) => {
        if (!cancelled) setError(String(reason));
      });
    return () => { cancelled = true; };
  }, [cwd]);

  if (!status || status.state === "enforced") return null;

  const canSetup = status.state === "setup_required" && status.setup_available;
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
            : (status.reason ?? "This platform sandbox is not available on this machine.")}
        >
          {canSetup && (
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                setBusy(true);
                setError("");
                void getBridge()
                  .then((bridge) => {
                    if (!bridge.setupLocalSandbox) throw new Error("Sandbox setup is unavailable");
                    return bridge.setupLocalSandbox(cwd);
                  })
                  .then(setStatus)
                  .catch((reason) => setError(String(reason)))
                  .finally(() => setBusy(false));
              }}
              className="flex shrink-0 items-center gap-1.5 rounded-lg bg-accent px-2.5 py-1.5 text-xs font-medium text-white transition hover:brightness-110 disabled:opacity-60"
            >
              {busy && <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" />}
              {busy ? "Waiting for Windows…" : "Enable sandbox"}
            </button>
          )}
        </Row>
        {error && <p className="px-3.5 py-2.5 text-xs text-danger">{error}</p>}
      </Card>
    </div>
  );
}
