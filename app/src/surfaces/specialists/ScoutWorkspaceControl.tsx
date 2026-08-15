import { ChevronDown, Plus } from "lucide-react";
import type { ScoutWorkspace, SpecialistOrganization } from "../../lib/specialistCloud";
import { cn } from "../../lib/cn";

export interface ScoutWorkspaceNoticeValue {
  tone: "success" | "error";
  message: string;
}

export function ScoutWorkspaceControl({
  organizationId,
  organizations,
  workspaces,
  workspaceId,
  serverReady,
  bound,
  creating,
  onSelect,
  onCreate,
}: {
  organizationId?: string;
  organizations: SpecialistOrganization[];
  workspaces: ScoutWorkspace[];
  workspaceId?: string;
  serverReady: boolean;
  bound: boolean;
  creating: boolean;
  onSelect: (workspaceId?: string) => void;
  onCreate: () => void;
}) {
  if (!organizationId || organizations.length === 0 || !serverReady) return null;
  if (workspaces.length > 0) {
    return (
      <label className="relative hidden md:block">
        <span className="sr-only">Workspace</span>
        <select
          value={workspaceId ?? ""}
          onChange={(event) => onSelect(event.target.value || undefined)}
          disabled={bound}
          title={bound ? "Start a new specialist conversation to change workspace" : undefined}
          className="h-9 appearance-none rounded-xl bg-bg-secondary pl-8 pr-8 text-xs font-medium text-ink-secondary outline-none transition focus:ring-2 focus:ring-accent/20"
        >
          <option value="">Choose workspace…</option>
          {workspaces.map((workspace) => (
            <option key={workspace.id} value={workspace.id}>{workspace.display_name}</option>
          ))}
        </select>
        <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 size-3.5 -translate-y-1/2 text-ink-faint" />
      </label>
    );
  }
  if (bound) return null;
  const organization = organizations.find((item) => item.id === organizationId);
  const canCreate = organization
    ? ["owner", "admin"].includes(organization.role.toLowerCase())
    : false;
  if (!canCreate) {
    return (
      <span className="hidden text-xs font-medium text-ink-muted md:inline">
        Ask an organization admin to create a workspace
      </span>
    );
  }
  return (
    <button
      type="button"
      onClick={onCreate}
      disabled={creating}
      className="flex h-9 items-center gap-1.5 rounded-xl bg-accent px-3 text-xs font-semibold text-on-accent transition hover:bg-accent/90 disabled:opacity-50"
    >
      <Plus className="size-3.5" />
      {creating ? "Creating…" : "Create workspace"}
    </button>
  );
}

export function ScoutWorkspaceNotice({
  notice,
  onDismiss,
}: {
  notice: ScoutWorkspaceNoticeValue;
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
