import { Plus } from "lucide-react";
import type { SpecialistOrganization } from "../../lib/specialistCloud";
import { cn } from "../../lib/cn";

export interface CompanyScoutSetupNoticeValue {
  tone: "success" | "error";
  message: string;
}

export function CompanyScoutSetupControl({
  organizationId,
  organizations,
  companyScoutReady,
  serverReady,
  bound,
  settingUp,
  onSetup,
}: {
  organizationId?: string;
  organizations: SpecialistOrganization[];
  companyScoutReady: boolean;
  serverReady: boolean;
  bound: boolean;
  settingUp: boolean;
  onSetup: () => void;
}) {
  if (!organizationId || organizations.length === 0 || !serverReady) return null;
  if (companyScoutReady || bound) return null;
  const organization = organizations.find((item) => item.id === organizationId);
  const canSetup = organization
    ? ["owner", "admin"].includes(organization.role.toLowerCase())
    : false;
  if (!canSetup) {
    return (
      <span className="hidden text-xs font-medium text-ink-muted md:inline">
        Ask a company admin to set up Company Scout
      </span>
    );
  }
  return (
    <button
      type="button"
      onClick={onSetup}
      disabled={settingUp}
      className="flex h-9 items-center gap-1.5 rounded-xl bg-accent px-3 text-xs font-semibold text-on-accent transition hover:bg-accent/90 disabled:opacity-50"
    >
      <Plus className="size-3.5" />
      {settingUp ? "Setting up…" : "Set up Company Scout"}
    </button>
  );
}

export function CompanyScoutSetupNotice({
  notice,
  onDismiss,
}: {
  notice: CompanyScoutSetupNoticeValue;
  onDismiss: () => void;
}) {
  return (
    <div
      role={notice.tone === "error" ? "alert" : "status"}
      className={cn(
        "mx-5 mb-2 flex shrink-0 items-center gap-3 rounded-xl border px-3 py-2 text-xs",
        notice.tone === "error"
          ? "border-danger/25 bg-danger/5 text-danger"
          : "border-success/25 bg-success/5 text-success",
      )}
    >
      <span className="min-w-0 flex-1">{notice.message}</span>
      <button
        type="button"
        onClick={onDismiss}
        className="font-semibold opacity-70 transition hover:opacity-100"
      >
        Dismiss
      </button>
    </div>
  );
}
