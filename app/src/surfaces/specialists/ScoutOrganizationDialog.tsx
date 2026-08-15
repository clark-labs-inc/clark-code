export function ScoutOrganizationDialog({
  name,
  domain,
  creating,
  onNameChange,
  onDomainChange,
  onCancel,
  onCreate,
}: {
  name: string;
  domain: string;
  creating: boolean;
  onNameChange: (value: string) => void;
  onDomainChange: (value: string) => void;
  onCancel: () => void;
  onCreate: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/40 p-4" role="presentation">
      <form
        role="dialog"
        aria-modal="true"
        aria-labelledby="create-scout-organization-title"
        onSubmit={(event) => {
          event.preventDefault();
          onCreate();
        }}
        className="w-full max-w-md rounded-2xl border border-border bg-bg-elevated p-5 shadow-xl"
      >
        <h2 id="create-scout-organization-title" className="text-base font-semibold text-ink">
          Create Scout organization
        </h2>
        <p className="mt-1 text-xs leading-5 text-ink-muted">
          This creates the explicit tenant boundary. Scout will still wait for you to create a workspace and press Start run.
        </p>
        <label className="mt-4 block text-xs font-medium text-ink-secondary">
          Organization name
          <input
            value={name}
            onChange={(event) => onNameChange(event.target.value)}
            autoFocus
            className="mt-1.5 h-10 w-full rounded-xl border border-border bg-bg px-3 text-sm text-ink outline-none focus:border-accent"
          />
        </label>
        <label className="mt-3 block text-xs font-medium text-ink-secondary">
          Company domain
          <input
            value={domain}
            onChange={(event) => onDomainChange(event.target.value)}
            placeholder="example.com"
            className="mt-1.5 h-10 w-full rounded-xl border border-border bg-bg px-3 text-sm text-ink outline-none focus:border-accent"
          />
        </label>
        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-xl px-3 py-2 text-xs font-medium text-ink-muted hover:bg-bg-hover"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={creating || !name.trim() || !domain.trim()}
            className="rounded-xl bg-accent px-3 py-2 text-xs font-semibold text-on-accent disabled:opacity-50"
          >
            {creating ? "Creating…" : "Create organization"}
          </button>
        </div>
      </form>
    </div>
  );
}
