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
import { MacPermissionGuide } from "./MacPermissionGuide";
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
  if (!status) return "Checking the native service and OS permissions…";
  if (!status.supported) return `Native computer use is unavailable on ${status.platform}.`;
  if (!status.service_ready) return status.detail || "The signed computer-use service is unavailable.";
  if (status.readiness === "needs_permission") {
    return `${status.permission_owner?.display_name || "the agent Computer Use"} needs ${status.platform === "macos" ? "macOS privacy" : "desktop capture and input"} access.`;
  }
  if (status.readiness === "restart_required") return "The service must restart to use its new privacy grant.";
  return status.platform === "macos"
    ? "The signed computer-use service is ready."
    : "The isolated computer-use service is ready.";
}

export function computerUseRepairMessage(status: ComputerUsePlatformStatus): string {
  const owner = status.permission_owner?.display_name || "the agent Computer Use";
  if (status.platform === "macos") {
    return `Grant access to ${owner}. Existing Clark Code privacy grants do not transfer to the separately identified service.`;
  }
  if (status.platform === "windows") {
    return "Unlock the signed-in desktop session and run Clark Code interactively, then retry access. A Windows service or secure desktop cannot supply observable user input.";
  }
  return "Use an active X11 or XWayland desktop session, then retry access. Existing portal sessions do not transfer to the isolated service.";
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
          service_ready: false,
          readiness: "unsupported",
          permission_owner: null,
          detail: "Native computer use is available only inside Clark Code desktop host.",
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

  const canEnable = status?.supported === true && status.service_ready;
  const permissions = status?.permissions;

  return (
    <div className="space-y-6">
      <div>
        <GroupLabel>Computer use</GroupLabel>
        <Card>
          <Row
            icon={<MousePointer2 className="size-4" />}
            name="Allow computer use"
            sub="Lets the agent propose actions in ordinary desktop apps. Every action still passes native target, freshness, risk, and approval checks."
          >
            <Toggle
              on={enabled && canEnable}
              disabled={!canEnable}
              onClick={() => setLocalSettings({ computerUseEnabled: !enabled })}
              label="Allow computer use"
            />
          </Row>
          <Row
            icon={status?.service_ready ? <ShieldCheck className="size-4 text-success" /> : <AlertTriangle className="size-4 text-warning" />}
            name="Native service"
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

      {status?.supported && status.service_ready && (
        <div>
          <GroupLabel>{status.platform === "macos" ? "macOS privacy" : "Platform access"}</GroupLabel>
          <Card>
            {status.permission_owner && (
              <Row
                name="Permission owner"
                sub={`${status.permission_owner.display_name} · ${status.permission_owner.bundle_id}`}
              />
            )}
            <Row name={status.platform === "macos" ? "Accessibility" : "Desktop input"} sub="Required for trusted UI inspection and input">
              <PermissionMark granted={permissions?.accessibility === true} />
            </Row>
            <Row
              name={status.platform === "macos" ? "Screen Recording" : "Screen capture"}
              sub={status.platform === "macos"
                ? "Required for window screenshots; macOS may require an app restart"
                : "Required for fresh window screenshots from the active desktop session"}
            >
              <PermissionMark granted={permissions?.screen_recording === true} />
            </Row>
            {permissions?.screen_recording_restart_required && (
              <Row
                icon={<AlertTriangle className="size-4 text-warning" />}
                name="Restart required"
                sub="The platform recorded the screen-capture grant after this process launched."
              />
            )}
            {status.platform === "linux" && (
              <Row
                name="Session compatibility"
                sub="Window targeting and input currently cover X11 and XWayland apps. Native Wayland-only apps require a portal handoff."
              />
            )}
            {(!permissions?.accessibility || !permissions?.screen_recording) && (
              <Row
                name={status.platform === "macos" ? "Privacy setup" : "Access repair"}
                sub={computerUseRepairMessage(status)}
              >
                <button
                  type="button"
                  onClick={() => void (status.platform === "macos" ? requestPermissions() : refresh())}
                  disabled={working === "permissions"}
                  className="rounded-lg border border-border px-2.5 py-1.5 text-xs font-medium text-ink transition hover:bg-bg-hover disabled:opacity-50"
                >
                  {status.platform === "macos"
                    ? working === "permissions" ? "Opening…" : "Request permissions"
                    : "Retry access"}
                </button>
              </Row>
            )}
          </Card>
          {status.platform === "macos" && permissions && (!permissions.accessibility || !permissions.screen_recording) && (
            <div className="mt-3">
              <MacPermissionGuide
                ownerName={status.permission_owner?.display_name || "the agent Computer Use"}
                accessibilityGranted={permissions.accessibility}
                screenRecordingGranted={permissions.screen_recording}
                working={working === "permissions"}
                onRequestPermissions={() => void requestPermissions()}
              />
            </div>
          )}
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
