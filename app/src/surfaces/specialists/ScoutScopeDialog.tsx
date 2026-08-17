import { Building2, Map, Plus, X } from "lucide-react";
import type { SpecialistOrganization } from "../../lib/specialistCloud";

export function ScoutScopeDialog({
  organizations,
  companyScoutReady,
  organizationId,
  loading,
  settingUpCompanyScout,
  onSelectOrganization,
  onCreateOrganization,
  onSetupCompanyScout,
  onClose,
}: {
  organizations: SpecialistOrganization[];
  companyScoutReady: boolean;
  organizationId?: string;
  loading: boolean;
  settingUpCompanyScout: boolean;
  onSelectOrganization: (organizationId?: string) => void;
  onCreateOrganization: () => void;
  onSetupCompanyScout: () => void;
  onClose: () => void;
}) {
  const organization = organizations.find((item) => item.id === organizationId);
  const canSetupCompanyScout = organization
    ? ["owner", "admin"].includes(organization.role.toLowerCase())
    : false;

  return (
    <div
      className="fixed inset-0 z-50 grid place-items-center bg-black/40 p-4"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <section
        role="dialog"
        aria-modal="true"
        aria-labelledby="scout-scope-title"
        className="w-full max-w-md rounded-2xl border border-border bg-bg-elevated p-5 shadow-xl"
      >
        <div className="flex items-start gap-3">
          <div className="grid size-9 shrink-0 place-items-center rounded-xl bg-accent-subtle text-accent">
            <Map className="size-4" />
          </div>
          <div className="min-w-0 flex-1">
            <h2 id="scout-scope-title" className="text-base font-semibold text-ink">
              Choose company
            </h2>
            <p className="mt-1 text-xs leading-5 text-ink-muted">
              Company Scout builds one shared map across authorized Git, cloud, data, identity, delivery, and observability systems. Choose its execution instance separately beside the company chip.
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close company chooser"
            className="grid size-8 shrink-0 place-items-center rounded-lg text-ink-muted transition hover:bg-bg-hover hover:text-ink"
          >
            <X className="size-4" />
          </button>
        </div>

        <label className="mt-5 block text-xs font-medium text-ink-secondary">
          Company
          <span className="relative mt-1.5 block">
            <Building2 className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-ink-muted" />
            <select
              autoFocus
              value={organizationId ?? ""}
              onChange={(event) => onSelectOrganization(event.target.value || undefined)}
              className="h-11 w-full appearance-none rounded-xl border border-border bg-bg pl-10 pr-3 text-sm text-ink outline-none transition focus:border-accent"
            >
              <option value="">Choose company…</option>
              {organizations.map((item) => (
                <option key={item.id} value={item.id}>{item.name}</option>
              ))}
            </select>
          </span>
        </label>

        <button
          type="button"
          onClick={onCreateOrganization}
          className="mt-2 flex items-center gap-1.5 text-xs font-semibold text-accent hover:text-accent-hover"
        >
          <Plus className="size-3.5" /> Create company
        </button>

        {organizationId && (
          <div className="mt-5 rounded-xl border border-border-subtle bg-bg-secondary px-3 py-3 text-xs leading-5 text-ink-muted">
            {loading
              ? "Loading the company map…"
              : companyScoutReady
                ? `Company Scout maps all connected systems for ${organization?.name ?? "this company"}.`
                : "Company Scout has not been set up for this company yet."}
          </div>
        )}

        {organizationId && !loading && !companyScoutReady && (
          canSetupCompanyScout ? (
            <button
              type="button"
              onClick={onSetupCompanyScout}
              disabled={settingUpCompanyScout}
              className="mt-3 flex h-9 items-center gap-1.5 rounded-xl bg-accent px-3 text-xs font-semibold text-on-accent transition hover:bg-accent-hover disabled:opacity-50"
            >
              <Plus className="size-3.5" />
              {settingUpCompanyScout ? "Setting up…" : "Set up Company Scout"}
            </button>
          ) : (
            <p className="mt-3 text-xs text-ink-muted">
              Ask a company admin to set up Company Scout.
            </p>
          )
        )}

        <div className="mt-6 flex justify-end">
          <button
            type="button"
            onClick={onClose}
            className="rounded-xl bg-bg-tertiary px-3 py-2 text-xs font-semibold text-ink-secondary transition hover:bg-bg-hover"
          >
            Done
          </button>
        </div>
      </section>
    </div>
  );
}
