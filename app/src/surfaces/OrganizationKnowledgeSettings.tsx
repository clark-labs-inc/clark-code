import { useEffect, useState } from "react";
import { Building2, FolderGit2, Loader2, LockKeyhole } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import { cloudCreds } from "../lib/cloudHistory";
import {
  loadOrganizationKnowledgeStatus,
  organizationForRepository,
  setOrganizationForRepository,
  type OrganizationKnowledgeStatus,
} from "../lib/organizationKnowledge";
import {
  projectKnowledgeEnabled,
  refreshRepositoryIdentity,
  setProjectKnowledgeEnabled,
  type RepositoryIdentity,
} from "../lib/repositoryKnowledge";

/** Explicit, repository-scoped consent for contributing local Git history. */
export function OrganizationKnowledgeSettings() {
  const auth = useSessionStore((state) => state.auth);
  const cwd = useSessionStore((state) => state.localSettings.cwd);
  const [status, setStatus] = useState<OrganizationKnowledgeStatus | null>(null);
  const [repository, setRepository] = useState<RepositoryIdentity | null>(null);
  const [selection, setSelection] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [enabled, setEnabled] = useState(projectKnowledgeEnabled);

  useEffect(() => {
    const creds = cloudCreds(auth);
    if (!creds || !cwd.trim() || !enabled) {
      setStatus(null);
      setRepository(null);
      setSelection("");
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError("");
    void Promise.all([
      loadOrganizationKnowledgeStatus(creds),
      refreshRepositoryIdentity(cwd),
    ])
      .then(([nextStatus, nextRepository]) => {
        if (cancelled) return;
        setStatus(nextStatus);
        setRepository(nextRepository);
        if (!nextRepository) {
          setSelection("");
          return;
        }
        const saved = organizationForRepository(nextRepository.fingerprint) ?? "";
        const valid = nextStatus.organizations.some(
          (organization) => organization.organization_id === saved,
        );
        if (saved && !valid) {
          setOrganizationForRepository(nextRepository.fingerprint, null);
        }
        setSelection(valid ? saved : "");
      })
      .catch((reason) => {
        if (!cancelled) setError(String(reason));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [auth, cwd, enabled]);

  const disabled = !enabled;
  const organizations = status?.organizations ?? [];
  const unavailable = disabled || !repository || organizations.length === 0;
  const detail = disabled
    ? "Turn on Project knowledge above first."
    : !repository
      ? "The selected folder is not a Git repository."
      : organizations.length === 0
        ? "No organization you belong to has enabled organizational memory."
        : "Private by default. Choose one organization to contribute this repository's bounded commit evidence.";

  return (
    <div className="space-y-6">
      <div>
        <div className="mb-2 text-xs font-semibold uppercase tracking-wider text-ink-faint">
          Project knowledge
        </div>
        <div className="flex items-center gap-3 rounded-xl border border-border-subtle bg-bg-elevated/40 px-3.5 py-3">
          <FolderGit2 className="size-4 shrink-0 text-ink-muted" />
          <div className="min-w-0 flex-1">
            <div className="text-sm text-ink">Index repository history</div>
            <div className="mt-0.5 text-xs text-ink-faint">
              Sync Git identity and bounded commit history for repositories inside the selected folder.
            </div>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={enabled}
            aria-label="Sync project knowledge"
            onClick={() => {
              const next = !enabled;
              setProjectKnowledgeEnabled(next);
              setEnabled(next);
            }}
            className={`relative h-[18px] w-8 shrink-0 rounded-full transition-colors ${
              enabled ? "bg-accent" : "bg-bg-tertiary"
            }`}
          >
            <span className={`absolute top-0.5 size-[14px] rounded-full bg-white shadow-sm transition-all ${
              enabled ? "left-[15px]" : "left-0.5"
            }`} />
          </button>
        </div>
      </div>
      {auth && (
      <div>
      <div className="mb-2 text-xs font-semibold uppercase tracking-wider text-ink-faint">
        Organization knowledge
      </div>
      <div className="rounded-xl border border-border-subtle bg-bg-elevated/40 px-3.5 py-3">
        <div className="flex items-center gap-3">
          <Building2 className="size-4 shrink-0 text-ink-muted" />
          <div className="min-w-0 flex-1">
            <div className="text-sm text-ink">Contribute this repository</div>
            <div className="mt-0.5 text-xs text-ink-faint">{detail}</div>
          </div>
          {loading ? (
            <Loader2 className="size-4 animate-spin text-ink-muted" aria-label="Loading organizations" />
          ) : (
            <select
              aria-label="Organization for this repository"
              value={selection}
              disabled={unavailable}
              onChange={(event) => {
                if (!repository) return;
                const organizationId = event.target.value;
                setOrganizationForRepository(
                  repository.fingerprint,
                  organizationId || null,
                );
                setSelection(organizationId);
              }}
              className="max-w-48 rounded-lg border border-border bg-bg px-2.5 py-1.5 text-xs text-ink outline-none disabled:opacity-50"
            >
              <option value="">Private — personal only</option>
              {organizations.map((organization) => (
                <option key={organization.organization_id} value={organization.organization_id}>
                  {organization.name}
                </option>
              ))}
            </select>
          )}
        </div>
        {selection && (
          <p className="mt-2 flex items-center gap-1.5 text-xs text-ink-faint">
            <LockKeyhole className="size-3.5" />
            Only active members can retrieve extracted claims; raw repository files are not uploaded.
          </p>
        )}
        {error && <p className="mt-2 text-xs text-danger">Could not load organizations.</p>}
      </div>
      </div>
      )}
    </div>
  );
}
