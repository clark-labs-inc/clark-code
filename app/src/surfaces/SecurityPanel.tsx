import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { RefreshCw, ShieldCheck, X } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { projectName } from "../lib/localAgent";
import { cn } from "../lib/cn";
import { cloudCreds } from "../lib/cloudHistory";
import { codeKeyAccountBinding } from "../lib/account";
import {
  inspectSecurityRepository,
  loadSecurityOrganizations,
  registerSecurityRepository,
  selectedSecurityOrganization,
  selectSecurityOrganization,
  syncSecurityScans,
  type SecurityOrganization,
  type SecurityRepositoryRegistration,
  type SecurityCloudSyncResult,
} from "../lib/securityCloud";
import type {
  SecurityScanRecord,
  SecuritySeverity,
} from "../core-bridge/types";

const SEVERITY_TONE: Record<SecuritySeverity, string> = {
  critical: "border-danger/50 text-danger",
  high: "border-warning/50 text-warning",
  medium: "border-info/50 text-info",
  low: "border-border text-ink-muted",
};

type SecurityCloudState =
  | { status: "unavailable" }
  | { status: "loading" }
  | { status: "not_git" }
  | {
      status: "choose";
      fingerprint: string;
      organizations: SecurityOrganization[];
    }
  | {
      status: "ready";
      organization: SecurityOrganization;
      registration: SecurityRepositoryRegistration;
      sync?: SecurityCloudSyncResult;
      syncError?: string;
    }
  | { status: "error"; message: string };

export function summarizeSecurityScan(record: SecurityScanRecord) {
  return {
    sealed: Boolean(record.seal),
    findings: record.seal?.findings.length ?? 0,
    reviewed: record.seal?.reviewedFiles ?? record.bundle.coverage.length,
    excluded:
      record.seal?.excludedFiles
      ?? record.bundle.coverage.filter((row) => row.status === "excluded").length,
    supporting:
      record.seal?.supportingFiles ?? record.bundle.supportingCoverage.length,
  };
}

export function previouslySelectedSecurityOrganization(
  organizations: SecurityOrganization[],
  selectedOrganizationId: string | null,
) {
  return organizations.find((organization) => organization.id === selectedOrganizationId);
}

export function SecurityButton() {
  const bridge = useSessionStore((state) => state.bridge);
  const auth = useSessionStore((state) => state.auth);
  const cwd = useSessionStore(
    (state) => state.activeProjectRoot ?? state.localSettings.cwd,
  );
  const creds = useMemo(() => cloudCreds(auth), [auth]);
  const accountScope = codeKeyAccountBinding(auth);
  const [open, setOpen] = useState(false);
  const [records, setRecords] = useState<SecurityScanRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cloud, setCloud] = useState<SecurityCloudState>({ status: "unavailable" });
  const wrapRef = useRef<HTMLDivElement>(null);
  const contextKey = `${accountScope ?? "signed-out"}\u0000${cwd.trim()}`;
  const contextRef = useRef(contextKey);
  contextRef.current = contextKey;

  useEffect(() => {
    setOpen(false);
    setRecords([]);
    setLoading(false);
    setError(null);
    setCloud({ status: "unavailable" });
  }, [contextKey]);

  const load = useCallback(async () => {
    const activeBridge = bridge;
    if (!cwd || !activeBridge?.listSecurityScans) return;
    const requestContext = contextKey;
    setLoading(true);
    setError(null);
    try {
      const nextRecords = await activeBridge.listSecurityScans(cwd);
      if (contextRef.current === requestContext) setRecords(nextRecords);
    } catch (cause) {
      if (contextRef.current === requestContext) {
        setError(cause instanceof Error ? cause.message : String(cause));
      }
    } finally {
      if (contextRef.current === requestContext) setLoading(false);
    }
  }, [bridge, contextKey, cwd]);

  const connectRepository = useCallback(async (
    organization: SecurityOrganization,
    fingerprint: string,
  ) => {
    if (!creds || !cwd) return;
    const requestContext = contextKey;
    selectSecurityOrganization(fingerprint, organization.id, accountScope);
    setCloud({ status: "loading" });
    try {
      const registration = await registerSecurityRepository(
        creds,
        organization.id,
        cwd,
      );
      if (contextRef.current !== requestContext) return;
      try {
        const sync = await syncSecurityScans(
          creds,
          organization.id,
          registration,
          cwd,
        );
        if (contextRef.current !== requestContext) return;
        setCloud({ status: "ready", organization, registration, sync });
      } catch (cause) {
        if (contextRef.current !== requestContext) return;
        setCloud({
          status: "ready",
          organization,
          registration,
          syncError: cause instanceof Error ? cause.message : String(cause),
        });
      }
    } catch (cause) {
      if (contextRef.current !== requestContext) return;
      setCloud({
        status: "error",
        message: cause instanceof Error ? cause.message : String(cause),
      });
    }
  }, [accountScope, contextKey, creds, cwd]);

  const loadCloud = useCallback(async () => {
    if (!creds || !cwd) {
      setCloud({ status: "unavailable" });
      return;
    }
    const requestContext = contextKey;
    setCloud({ status: "loading" });
    try {
      const [repository, organizations] = await Promise.all([
        inspectSecurityRepository(cwd),
        loadSecurityOrganizations(creds),
      ]);
      if (contextRef.current !== requestContext) return;
      if (!repository) {
        setCloud({ status: "not_git" });
        return;
      }
      if (organizations.length === 0) {
        setCloud({
          status: "error",
          message: "No active Clark workspace is available for this account.",
        });
        return;
      }
      const selected = selectedSecurityOrganization(repository.fingerprint, accountScope);
      const organization = previouslySelectedSecurityOrganization(organizations, selected);
      if (!organization) {
        setCloud({
          status: "choose",
          fingerprint: repository.fingerprint,
          organizations,
        });
        return;
      }
      await connectRepository(organization, repository.fingerprint);
    } catch (cause) {
      if (contextRef.current !== requestContext) return;
      setCloud({
        status: "error",
        message: cause instanceof Error ? cause.message : String(cause),
      });
    }
  }, [accountScope, connectRepository, contextKey, creds, cwd]);

  useEffect(() => {
    if (open) {
      void load();
      void loadCloud();
    }
  }, [load, loadCloud, open]);

  useEffect(() => {
    if (!open) return;
    const onDown = (event: MouseEvent) => {
      if (!wrapRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={wrapRef} className="relative">
      <button
        onClick={() => setOpen((current) => !current)}
        aria-label={open ? "Hide Security scans" : "Show Security scans"}
        title="Security scan history"
        className={cn(
          "grid size-8 place-items-center rounded-lg transition",
          open
            ? "bg-accent-soft text-accent"
            : "text-ink-muted hover:bg-accent-subtle hover:text-accent",
        )}
      >
        <ShieldCheck className="size-4" />
      </button>
      {open && (
        <SecurityPopover
          cwd={cwd}
          records={records}
          loading={loading}
          error={error}
          cloud={cloud}
          onReload={load}
          onConnect={connectRepository}
          onClose={() => setOpen(false)}
        />
      )}
    </div>
  );
}

function SecurityPopover({
  cwd,
  records,
  loading,
  error,
  cloud,
  onReload,
  onConnect,
  onClose,
}: {
  cwd: string;
  records: SecurityScanRecord[];
  loading: boolean;
  error: string | null;
  cloud: SecurityCloudState;
  onReload: () => Promise<void>;
  onConnect: (
    organization: SecurityOrganization,
    fingerprint: string,
  ) => Promise<void>;
  onClose: () => void;
}) {
  return (
    <div className="popover-surface absolute right-0 top-10 z-50 flex max-h-[72vh] w-[30rem] flex-col overflow-hidden rounded-xl border border-border bg-bg-elevated shadow-xl">
      <header className="flex items-center gap-2 border-b border-border-subtle px-3 py-2.5">
        <ShieldCheck className="size-4 shrink-0 text-ink-muted" />
        <div className="min-w-0">
          <p className="text-sm font-medium text-ink">Security</p>
          <p className="truncate text-xs text-ink-faint">
            {cwd ? projectName(cwd) : "No project selected"}
          </p>
        </div>
        <button
          onClick={() => void onReload()}
          disabled={loading}
          aria-label="Reload Security scans"
          className="ml-auto grid size-7 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink disabled:opacity-50"
        >
          <RefreshCw
            className={cn(
              "size-3.5",
              loading && "animate-[spin_1s_linear_infinite]",
            )}
          />
        </button>
        <button
          onClick={onClose}
          aria-label="Close"
          className="grid size-7 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink"
        >
          <X className="size-3.5" />
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-3">
        <SecurityCloudStatus state={cloud} onConnect={onConnect} />
        {error ? (
          <p className="py-6 text-center text-xs text-danger">{error}</p>
        ) : loading && records.length === 0 ? (
          <p className="py-6 text-center text-xs text-ink-faint">
            Reading Security artifacts…
          </p>
        ) : records.length === 0 ? (
          <div className="rounded-lg border border-dashed border-border-subtle px-4 py-5 text-center">
            <p className="text-sm font-medium text-ink-secondary">No scans yet</p>
            <p className="mt-1 text-xs text-ink-muted">
              Run <code className="font-mono">/security</code>,{" "}
              <code className="font-mono">/security-diff</code>, or{" "}
              <code className="font-mono">/security-deep</code>.
            </p>
          </div>
        ) : (
          <div className="space-y-2">
            {records.map((record) => (
              <ScanCard key={`${record.path}:${record.seal?.bundleDigest ?? "open"}`} record={record} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function SecurityCloudStatus({
  state,
  onConnect,
}: {
  state: SecurityCloudState;
  onConnect: (
    organization: SecurityOrganization,
    fingerprint: string,
  ) => Promise<void>;
}) {
  if (state.status === "unavailable") return null;
  if (state.status === "loading") {
    return (
      <p className="mb-3 rounded-lg border border-border-subtle px-3 py-2 text-xs text-ink-muted">
        Connecting this repository to Clark Security…
      </p>
    );
  }
  if (state.status === "not_git") {
    return (
      <p className="mb-3 rounded-lg border border-border-subtle px-3 py-2 text-xs text-ink-muted">
        Cloud rescans require a Git repository.
      </p>
    );
  }
  if (state.status === "error") {
    return (
      <p className="mb-3 rounded-lg border border-danger/30 px-3 py-2 text-xs text-danger">
        Cloud Security sync: {state.message}
      </p>
    );
  }
  if (state.status === "choose") {
    if (state.organizations.length === 1) {
      const organization = state.organizations[0];
      return (
        <div className="mb-3 rounded-lg border border-border-subtle px-3 py-2">
          <p className="text-xs font-medium text-ink-secondary">
            Connect this repository to {organization.name}?
          </p>
          <p className="mt-1 text-xs text-ink-muted">
            Clark will register this Git repository and sync sealed Security results while it is open.
          </p>
          <button
            type="button"
            onClick={() => void onConnect(organization, state.fingerprint)}
            className="mt-2 rounded-md bg-accent px-2.5 py-1.5 text-xs font-medium text-white transition hover:opacity-90"
          >
            Connect repository
          </button>
        </div>
      );
    }
    return (
      <label className="mb-3 block rounded-lg border border-border-subtle px-3 py-2">
        <span className="block text-xs font-medium text-ink-secondary">
          Security workspace
        </span>
        <select
          defaultValue=""
          onChange={(event) => {
            const organization = state.organizations.find(
              (item) => item.id === event.target.value,
            );
            if (organization) {
              void onConnect(organization, state.fingerprint);
            }
          }}
          className="mt-1 w-full rounded-md border border-border bg-bg px-2 py-1.5 text-xs text-ink"
        >
          <option value="" disabled>
            Choose where results belong
          </option>
          {state.organizations.map((organization) => (
            <option key={organization.id} value={organization.id}>
              {organization.name}
            </option>
          ))}
        </select>
      </label>
    );
  }
  const interval = state.registration.repositoryPolicy.scheduleIntervalMinutes;
  return (
    <div className="mb-3 rounded-lg border border-success/30 bg-success/5 px-3 py-2">
      <p className="text-xs font-medium text-success">
        Continuous scanning active · {state.organization.name}
      </p>
      <p className="mt-0.5 text-xs text-ink-muted">
        {state.registration.repository.githubManaged
          ? "GitHub access is connected; rescans run in the cloud."
          : "Sealed local scans sync automatically while this repository is open."}
        {interval ? ` Default cadence: every ${interval / 60} hours.` : ""}
      </p>
      {state.sync ? (
        <p className="mt-1 text-xs text-ink-muted">
          Local evidence: {state.sync.syncedCount} uploaded
          {state.sync.alreadySyncedCount
            ? ` · ${state.sync.alreadySyncedCount} already current`
            : ""}
          {state.sync.pendingCount ? ` · ${state.sync.pendingCount} queued` : ""}
          {state.sync.failedCount ? ` · ${state.sync.failedCount} will retry` : ""}
        </p>
      ) : null}
      {state.syncError ? (
        <p className="mt-1 text-xs text-warning">
          Local evidence will retry automatically: {state.syncError}
        </p>
      ) : null}
    </div>
  );
}

function ScanCard({ record }: { record: SecurityScanRecord }) {
  const summary = summarizeSecurityScan(record);
  return (
    <details className="group rounded-lg border border-border-subtle bg-bg-sunken/40 px-3 py-2.5">
      <summary className="cursor-pointer list-none">
        <div className="flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-xs font-semibold text-ink-secondary">
            {record.bundle.scanId}
          </span>
          <span className="rounded-full border border-border px-1.5 py-px text-xs uppercase text-ink-muted">
            {record.bundle.mode}
          </span>
          <span
            className={cn(
              "rounded-full border px-1.5 py-px text-xs",
              summary.sealed
                ? "border-success/40 text-success"
                : "border-warning/40 text-warning",
            )}
          >
            {summary.sealed ? "sealed" : "in progress"}
          </span>
        </div>
        <p className="mt-1 text-xs text-ink-faint">
          {summary.findings} findings · {summary.reviewed} reviewed ·{" "}
          {summary.excluded} excluded
          {summary.supporting > 0 ? ` · ${summary.supporting} supporting` : ""}
          {record.seal?.deepPasses
            ? ` · ${record.seal.deepPasses} deep passes`
            : ""}
        </p>
      </summary>

      {record.seal?.findings.length ? (
        <div className="mt-2 space-y-1.5 border-t border-border-subtle pt-2">
          {record.seal.findings.map((finding) => (
            <div key={finding.findingId} className="rounded-md border border-border-subtle p-2">
              <div className="flex items-center gap-2">
                <span className="font-mono text-xs text-ink-faint">
                  {finding.findingId}
                </span>
                <span
                  className={cn(
                    "ml-auto rounded-full border px-1.5 py-px text-xs uppercase",
                    SEVERITY_TONE[finding.severity],
                  )}
                >
                  {finding.severity}
                </span>
              </div>
              <p className="mt-1 text-xs text-ink-secondary">{finding.impact}</p>
              <p className="mt-1 truncate font-mono text-xs text-ink-faint">
                {finding.sourcePath}
              </p>
            </div>
          ))}
        </div>
      ) : null}
      <p className="mt-2 truncate border-t border-border-subtle pt-2 font-mono text-xs text-ink-faint">
        {record.path}
      </p>
    </details>
  );
}
