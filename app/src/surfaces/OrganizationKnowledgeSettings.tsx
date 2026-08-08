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
import { GroupLabel, Card, Row, Toggle } from "./settings/Primitives";
import { codeKeyAccountBinding } from "../lib/account";

/** Explicit, repository-scoped consent for contributing local Git history. */
export function OrganizationKnowledgeSettings() {
  const auth = useSessionStore((state) => state.auth);
  const accountScope = codeKeyAccountBinding(auth);
  const cwd = useSessionStore((state) => state.localSettings.cwd);
  const [status, setStatus] = useState<OrganizationKnowledgeStatus | null>(null);
  const [repository, setRepository] = useState<RepositoryIdentity | null>(null);
  const [selection, setSelection] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [enabled, setEnabled] = useState(() => projectKnowledgeEnabled(accountScope));

  useEffect(() => {
    setEnabled(projectKnowledgeEnabled(accountScope));
  }, [accountScope]);

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
      refreshRepositoryIdentity(cwd, accountScope),
    ])
      .then(([nextStatus, nextRepository]) => {
        if (cancelled) return;
        setStatus(nextStatus);
        setRepository(nextRepository);
        if (!nextRepository) {
          setSelection("");
          return;
        }
        const saved = organizationForRepository(nextRepository.fingerprint, accountScope) ?? "";
        const valid = nextStatus.organizations.some(
          (organization) => organization.organization_id === saved,
        );
        if (saved && !valid) {
          setOrganizationForRepository(nextRepository.fingerprint, null, accountScope);
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
  }, [accountScope, auth, cwd, enabled]);

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
        <GroupLabel>Project knowledge</GroupLabel>
        <Card>
          <Row
            icon={<FolderGit2 className="size-4" />}
            name="Index repository history"
            sub="Sync Git identity and bounded commit history for repositories inside the selected folder."
          >
            <Toggle
              on={enabled}
              onClick={() => {
                const next = !enabled;
                setProjectKnowledgeEnabled(next, accountScope);
                setEnabled(next);
              }}
              label="Sync project knowledge"
            />
          </Row>
        </Card>
      </div>
      {auth && (
        <div>
          <GroupLabel>Organization knowledge</GroupLabel>
          <Card>
            <Row
              icon={<Building2 className="size-4" />}
              name="Contribute this repository"
              sub={detail}
            >
              {loading ? (
                <Loader2 className="size-4 shrink-0 animate-[spin_1s_linear_infinite] text-ink-muted" aria-label="Loading organizations" />
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
                      accountScope,
                    );
                    setSelection(organizationId);
                  }}
                  className="max-w-48 shrink-0 rounded-lg border border-border bg-bg px-2.5 py-1.5 text-xs text-ink outline-none disabled:opacity-50"
                >
                  <option value="">Private — personal only</option>
                  {organizations.map((organization) => (
                    <option key={organization.organization_id} value={organization.organization_id}>
                      {organization.name}
                    </option>
                  ))}
                </select>
              )}
            </Row>
            {(selection || error) && (
              <div className="px-3.5 py-2.5">
                {selection && (
                  <p className="flex items-center gap-1.5 text-xs text-ink-faint">
                    <LockKeyhole className="size-3.5 shrink-0" />
                    Only active members can retrieve extracted claims; raw repository files are not uploaded.
                  </p>
                )}
                {error && <p className="mt-1 text-xs text-danger">Could not load organizations.</p>}
              </div>
            )}
          </Card>
        </div>
      )}
    </div>
  );
}
