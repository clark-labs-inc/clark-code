import { useCallback, useEffect, useState } from "react";
import {
  AlertTriangle,
  Check,
  Loader2,
  MousePointer2,
  RefreshCw,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";
import { getBridge } from "../../core-bridge/bridge";
import type {
  ComputerUseActionReceipt,
  ComputerUseAppApproval,
  ComputerUseApprovalSnapshot,
  ComputerUsePlatformStatus,
} from "../../core-bridge/bridge";
import { useSessionStore } from "../../store/sessionStore";
import { Card, GroupLabel, Row, Toggle } from "./Primitives";

const EMPTY_APPROVALS: ComputerUseApprovalSnapshot = { revision: 0, approvals: [] };

function shortDate(timestamp: number): string {
  if (!Number.isFinite(timestamp) || timestamp <= 0) return "Unknown";
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(timestamp));
}

function PermissionMark({ granted }: { granted: boolean }) {
  const Icon = granted ? Check : X;
  return (
    <span className={granted ? "inline-flex items-center gap-1 text-success" : "inline-flex items-center gap-1 text-warning"}>
      <Icon className="size-3.5" />
      {granted ? "Granted" : "Required"}
    </span>
  );
}

export function computerUseSupportMessage(status: ComputerUsePlatformStatus | null): string {
  if (!status) return "Checking native helper and OS permissions…";
  if (!status.supported) return `Native computer use is unavailable on ${status.platform}.`;
  if (!status.helper_ready) return status.detail || "The signed computer-use helper is unavailable.";
  return "The signed helper is ready.";
}

export function ComputerUseApprovalRows({
  approvals,
  working,
  onRevoke,
}: {
  approvals: ComputerUseAppApproval[];
  working: string | null;
  onRevoke: (identityKey: string) => void;
}) {
  if (approvals.length === 0) {
    return (
      <Row
        name="No durable approvals"
        sub="Apps appear here only after you choose an app-specific “always allow” decision."
      />
    );
  }
  return approvals.map((approval) => (
    <Row
      key={approval.identity_key}
      name={approval.app_name || approval.bundle_id}
      sub={`${approval.bundle_id}${approval.team_identifier ? ` · Team ${approval.team_identifier}` : ""} · Used ${shortDate(approval.last_used_at_ms)}`}
    >
      <button
        type="button"
        onClick={() => onRevoke(approval.identity_key)}
        disabled={working === approval.identity_key}
        aria-label={`Revoke ${approval.app_name || approval.bundle_id}`}
        className="grid size-8 shrink-0 place-items-center rounded-lg text-danger transition hover:bg-danger/10 disabled:opacity-50"
      >
        {working === approval.identity_key
          ? <Loader2 className="size-3.5 animate-spin" />
          : <Trash2 className="size-3.5" />}
      </button>
    </Row>
  ));
}

export function ComputerUseReceiptRows({
  receipts,
}: {
  receipts: ComputerUseActionReceipt[];
}) {
  const recent = receipts.slice(-5).reverse();
  if (recent.length === 0) {
    return (
      <Row
        name="No persisted receipts"
        sub="Typed text, control values, window titles, and element values are never stored here."
      />
    );
  }
  return recent.map((receipt) => (
    <Row
      key={receipt.receipt_id}
      name={`${receipt.action_kind.replaceAll("_", " ")} · ${receipt.outcome.replaceAll("_", " ")}`}
      sub={`${receipt.bundle_id} · ${receipt.payload_summary} · ${shortDate(receipt.completed_at_ms)}`}
    />
  ));
}

export function ComputerUseSection() {
  const enabled = useSessionStore((state) => state.localSettings.computerUseEnabled === true);
  const setLocalSettings = useSessionStore((state) => state.setLocalSettings);
  const [status, setStatus] = useState<ComputerUsePlatformStatus | null>(null);
  const [approvals, setApprovals] = useState(EMPTY_APPROVALS);
  const [receipts, setReceipts] = useState<ComputerUseActionReceipt[]>([]);
  const [loading, setLoading] = useState(true);
  const [working, setWorking] = useState<string | null>(null);
  const [confirmAll, setConfirmAll] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const bridge = await getBridge();
      if (
        !bridge.computerUsePlatformStatus
        || !bridge.computerUseApprovalSnapshot
        || !bridge.recentComputerUseReceipts
      ) {
        setStatus({
          supported: false,
          platform: "browser preview",
          helper_ready: false,
          detail: "Native computer use is available only inside the Clark Code desktop host.",
        });
        setApprovals(EMPTY_APPROVALS);
        setReceipts([]);
        return;
      }
      const [nextStatus, nextApprovals, nextReceipts] = await Promise.all([
        bridge.computerUsePlatformStatus(),
        bridge.computerUseApprovalSnapshot(),
        bridge.recentComputerUseReceipts(),
      ]);
      setStatus(nextStatus);
      setApprovals(nextApprovals);
      setReceipts(nextReceipts);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const requestPermissions = async () => {
    setWorking("permissions");
    setError(null);
    try {
      const bridge = await getBridge();
      if (!bridge.requestComputerUsePermissions) throw new Error("Native permission setup is unavailable.");
      await bridge.requestComputerUsePermissions();
      await refresh();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setWorking(null);
    }
  };

  const revoke = async (identityKey: string) => {
    setWorking(identityKey);
    setError(null);
    try {
      const bridge = await getBridge();
      if (!bridge.revokeComputerUseApproval) throw new Error("Native approval management is unavailable.");
      setApprovals(await bridge.revokeComputerUseApproval(identityKey));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setWorking(null);
    }
  };

  const revokeAll = async () => {
    if (!confirmAll) {
      setConfirmAll(true);
      return;
    }
    setWorking("all");
    setError(null);
    try {
      const bridge = await getBridge();
      if (!bridge.revokeAllComputerUseApprovals) throw new Error("Native approval management is unavailable.");
      setApprovals(await bridge.revokeAllComputerUseApprovals());
      setConfirmAll(false);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setWorking(null);
    }
  };

  const canEnable = status?.supported === true && status.helper_ready;
  const permissions = status?.permissions;

  return (
    <div className="space-y-6">
      <div>
        <GroupLabel>Computer use</GroupLabel>
        <Card>
          <Row
            icon={<MousePointer2 className="size-4" />}
            name="Allow computer use"
            sub="Lets Clark propose actions in ordinary Mac apps. Every action still passes native target, freshness, risk, and approval checks."
          >
            <Toggle
              on={enabled && canEnable}
              disabled={!canEnable}
              onClick={() => setLocalSettings({ computerUseEnabled: !enabled })}
              label="Allow computer use"
            />
          </Row>
          <Row
            icon={status?.helper_ready ? <ShieldCheck className="size-4 text-success" /> : <AlertTriangle className="size-4 text-warning" />}
            name="Native boundary"
            sub={computerUseSupportMessage(status)}
          >
            <button
              type="button"
              onClick={() => void refresh()}
              disabled={loading}
              aria-label="Refresh computer-use status"
              className="grid size-8 shrink-0 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:opacity-50"
            >
              {loading ? <Loader2 className="size-4 animate-spin" /> : <RefreshCw className="size-4" />}
            </button>
          </Row>
        </Card>
      </div>

      {status?.supported && status.helper_ready && (
        <div>
          <GroupLabel>macOS privacy</GroupLabel>
          <Card>
            <Row name="Accessibility" sub="Required for trusted UI inspection and input">
              <PermissionMark granted={permissions?.accessibility === true} />
            </Row>
            <Row name="Screen Recording" sub="Required for window screenshots; macOS may require an app restart">
              <PermissionMark granted={permissions?.screen_recording === true} />
            </Row>
            {permissions?.screen_recording_restart_required && (
              <Row
                icon={<AlertTriangle className="size-4 text-warning" />}
                name="Restart required"
                sub="macOS recorded the Screen Recording grant after this process launched."
              />
            )}
            {(!permissions?.accessibility || !permissions?.screen_recording) && (
              <Row name="Privacy setup" sub="macOS controls these grants; Clark cannot bypass or self-approve them.">
                <button
                  type="button"
                  onClick={() => void requestPermissions()}
                  disabled={working === "permissions"}
                  className="rounded-lg border border-border px-2.5 py-1.5 text-xs font-medium text-ink transition hover:bg-bg-hover disabled:opacity-50"
                >
                  {working === "permissions" ? "Opening…" : "Request permissions"}
                </button>
              </Row>
            )}
          </Card>
        </div>
      )}

      <div>
        <div className="mb-2 flex items-center justify-between px-0.5">
          <GroupLabel>Approved applications</GroupLabel>
          {approvals.approvals.length > 0 && (
            <button
              type="button"
              onClick={() => void revokeAll()}
              onBlur={() => setConfirmAll(false)}
              disabled={working === "all"}
              className="text-xs text-danger transition hover:underline disabled:opacity-50"
            >
              {working === "all" ? "Revoking…" : confirmAll ? "Confirm revoke all" : "Revoke all"}
            </button>
          )}
        </div>
        <Card>
          <ComputerUseApprovalRows
            approvals={approvals.approvals}
            working={working}
            onRevoke={(identityKey) => void revoke(identityKey)}
          />
        </Card>
        <p className="mt-2 px-0.5 text-xs leading-4 text-ink-faint">
          Grants are bound to the app’s code-signing identity, not its display name. Revocation waits for any earlier input lease to stop before it returns.
        </p>
      </div>

      <div>
        <GroupLabel>Recent redacted actions</GroupLabel>
        <Card>
          <ComputerUseReceiptRows receipts={receipts} />
        </Card>
      </div>

      {error && (
        <div role="alert" className="rounded-xl border border-danger/30 bg-danger/5 px-3.5 py-3 text-sm text-danger">
          {error}
        </div>
      )}
    </div>
  );
}
