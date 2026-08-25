import { useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  Check,
  Package,
  RefreshCw,
  Search,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";

import type {
  CoreBridge,
  InstalledSkillPack,
  ProjectInstructions,
  RemoteWorkerTarget,
  SkillCatalogEntry,
  SkillCatalogSnapshot,
  SkillPackReceipt,
  SkillPackScope,
} from "../core-bridge/bridge";
import { cn } from "../lib/cn";

interface SkillsPanelProps {
  open: boolean;
  bridge: CoreBridge | null;
  cwd: string;
  remote: RemoteWorkerTarget | null;
  catalog: SkillCatalogSnapshot | null;
  loading: boolean;
  error: string | null;
  onClose: () => void;
  onReload: () => Promise<SkillCatalogSnapshot | null>;
  onCatalog: (catalog: SkillCatalogSnapshot) => void;
  onSelect: (skill: SkillCatalogEntry) => void;
}

export function SkillsPanel(props: SkillsPanelProps) {
  const [query, setQuery] = useState("");
  const [instructions, setInstructions] = useState<ProjectInstructions | null>(null);
  const [packs, setPacks] = useState<InstalledSkillPack[]>([]);
  const [packId, setPackId] = useState("superpowers");
  const [sourcePath, setSourcePath] = useState("");
  const [scope, setScope] = useState<SkillPackScope>("project");
  const [working, setWorking] = useState(false);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [receipt, setReceipt] = useState<SkillPackReceipt | null>(null);

  useEffect(() => {
    if (!props.open) return;
    void props.bridge?.listInstructions?.(props.cwd, props.remote).then(setInstructions);
    void props.bridge?.listSkillPacks?.(props.cwd, props.remote).then(setPacks);
  }, [props.bridge, props.cwd, props.open, props.remote]);

  const skills = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const entries = props.catalog?.skills ?? [];
    if (!needle) return entries;
    return entries.filter((skill) =>
      [
        skill.name,
        skill.invocationName,
        skill.description,
        skill.scope,
        skill.origin,
        skill.source,
      ]
        .join(" ")
        .toLowerCase()
        .includes(needle),
    );
  }, [props.catalog, query]);

  if (!props.open) return null;

  const refreshPacks = async () => {
    const next = await props.bridge?.listSkillPacks?.(props.cwd, props.remote);
    if (next) setPacks(next);
  };

  const install = async () => {
    if (!props.bridge?.installSkillPack || !packId.trim() || !sourcePath.trim()) return;
    setWorking(true);
    setOperationError(null);
    try {
      const result = await props.bridge.installSkillPack(
        props.cwd,
        { packId: packId.trim(), sourcePath: sourcePath.trim(), scope },
        props.remote,
      );
      setReceipt(result.receipt);
      props.onCatalog(result.catalog);
      await refreshPacks();
    } catch (cause) {
      setOperationError(String(cause));
    } finally {
      setWorking(false);
    }
  };

  const uninstall = async (pack: InstalledSkillPack) => {
    if (!props.bridge?.uninstallSkillPack) return;
    setWorking(true);
    setOperationError(null);
    try {
      const result = await props.bridge.uninstallSkillPack(
        props.cwd,
        pack.packId,
        pack.scope,
        props.remote,
      );
      setReceipt(result.receipt);
      props.onCatalog(result.catalog);
      await refreshPacks();
    } catch (cause) {
      setOperationError(String(cause));
    } finally {
      setWorking(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-scrim p-6">
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Skills"
        className="popover-surface flex max-h-[86vh] w-full max-w-4xl flex-col overflow-hidden rounded-2xl bg-bg-elevated"
      >
        <header className="flex items-center gap-3 px-5 py-4">
          <Sparkles className="size-5 text-accent" />
          <div className="min-w-0 flex-1">
            <h2 className="font-semibold text-ink">Skills</h2>
            <p className="truncate text-xs text-ink-muted">
              {props.catalog
                ? `${props.catalog.skills.length} available from this environment · ${props.catalog.revision.slice(0, 20)}`
                : "Discovering this environment…"}
            </p>
          </div>
          <button
            type="button"
            onClick={() => void props.onReload()}
            disabled={props.loading}
            className="grid size-8 place-items-center rounded-lg text-ink-muted hover:bg-bg-hover hover:text-ink"
            aria-label="Reload skills"
          >
            <RefreshCw className={cn("size-4", props.loading && "animate-spin")} />
          </button>
          <button
            type="button"
            onClick={props.onClose}
            className="grid size-8 place-items-center rounded-lg text-ink-muted hover:bg-bg-hover hover:text-ink"
            aria-label="Close skills"
          >
            <X className="size-4" />
          </button>
        </header>

        <div className="grid min-h-0 flex-1 md:grid-cols-[minmax(0,1fr)_20rem]">
          <section className="flex min-h-0 flex-col bg-bg-secondary/35">
            <div className="relative m-3">
              <Search className="pointer-events-none absolute left-3 top-2.5 size-4 text-ink-faint" />
              <input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="Search skills, sources, or descriptions"
                className="h-9 w-full rounded-xl bg-bg-sunken px-9 text-sm text-ink outline-none focus:ring-2 focus:ring-accent/20"
              />
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3">
              {props.error && (
                <p className="mb-2 rounded-xl bg-danger/10 p-2 text-xs text-danger">{props.error}</p>
              )}
              {skills.map((skill) => (
                <button
                  key={skill.id}
                  type="button"
                  disabled={!skill.enabled}
                  onClick={() => props.onSelect(skill)}
                  className="mb-1 flex w-full items-start gap-3 rounded-xl px-3 py-2.5 text-left transition hover:bg-bg-hover disabled:cursor-not-allowed disabled:opacity-55"
                >
                  <Sparkles className="mt-0.5 size-4 shrink-0 text-ink-faint" />
                  <span className="min-w-0 flex-1">
                    <span className="flex flex-wrap items-center gap-1.5">
                      <span className="font-mono text-xs text-ink">${skill.invocationName}</span>
                      <span className="rounded bg-chip px-1.5 py-0.5 text-xs text-ink-faint">
                        {skill.scope} · {skill.origin}
                      </span>
                      {skill.hasNameCollision && (
                        <span className="rounded bg-warning/12 px-1.5 py-0.5 text-xs text-warning">
                          collision preserved
                        </span>
                      )}
                    </span>
                    <span className="mt-1 block text-xs leading-relaxed text-ink-muted">
                      {skill.description}
                    </span>
                    <span className="mt-1 block truncate font-mono text-xs text-ink-faint">
                      {skill.source}
                    </span>
                    {!skill.enabled && (
                      <span className="mt-1 block text-xs text-warning">{skill.disabledReason}</span>
                    )}
                  </span>
                </button>
              ))}
            </div>
          </section>

          <aside className="min-h-0 overflow-y-auto p-4">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-ink-faint">
              Managed packs
            </h3>
            <p className="mt-1 text-xs leading-relaxed text-ink-muted">
              Import a pack from this {props.remote ? "remote" : "local"} environment. the agent
              validates it, pins a content revision, and updates it atomically.
            </p>
            <div className="mt-3 space-y-2">
              <input
                value={packId}
                onChange={(event) => setPackId(event.target.value)}
                placeholder="Pack id"
                className="h-8 w-full rounded-lg border border-border-subtle bg-bg px-2.5 font-mono text-xs text-ink outline-none"
              />
              <input
                value={sourcePath}
                onChange={(event) => setSourcePath(event.target.value)}
                placeholder="/path/to/superpowers"
                className="h-8 w-full rounded-lg border border-border-subtle bg-bg px-2.5 font-mono text-xs text-ink outline-none"
              />
              <div className="flex gap-2">
                <select
                  value={scope}
                  onChange={(event) => setScope(event.target.value as SkillPackScope)}
                  className="h-8 flex-1 rounded-lg border border-border-subtle bg-bg px-2 text-xs text-ink"
                >
                  <option value="project">This project</option>
                  <option value="user">This environment</option>
                </select>
                <button
                  type="button"
                  disabled={working || !sourcePath.trim() || !packId.trim()}
                  onClick={() => void install()}
                  className="h-8 rounded-lg bg-accent px-3 text-xs font-medium text-on-accent disabled:opacity-50"
                >
                  Install / update
                </button>
              </div>
            </div>

            {operationError && (
              <p className="mt-2 rounded-lg bg-danger/10 p-2 text-xs text-danger">{operationError}</p>
            )}
            {receipt && (
              <div className="mt-2 rounded-xl bg-success/10 p-2 text-xs text-ink-secondary">
                <span className="flex items-center gap-1 font-medium text-success">
                  <Check className="size-3.5" />
                  {receipt.action} {receipt.packId}
                </span>
                <span className="mt-1 block font-mono text-xs text-ink-faint">
                  {receipt.revision ?? receipt.previousRevision}
                </span>
              </div>
            )}

            <div className="mt-3 space-y-1.5">
              {packs.map((pack) => (
                <div
                  key={`${pack.scope}:${pack.packId}`}
                  className="flex items-center gap-2 rounded-xl border border-border-subtle p-2"
                >
                  <Package className="size-4 text-ink-faint" />
                  <span className="min-w-0 flex-1">
                    <span className="block text-xs text-ink">{pack.packId}</span>
                    <span className="block truncate text-xs text-ink-faint">
                      {pack.scope} · {pack.skillCount} skills
                    </span>
                  </span>
                  <button
                    type="button"
                    disabled={working}
                    onClick={() => void uninstall(pack)}
                    aria-label={`Uninstall ${pack.packId}`}
                    className="grid size-7 place-items-center rounded-md text-ink-faint hover:bg-danger/10 hover:text-danger"
                  >
                    <Trash2 className="size-3.5" />
                  </button>
                </div>
              ))}
            </div>

            <h3 className="mt-5 text-xs font-semibold uppercase tracking-wide text-ink-faint">
              Instruction provenance
            </h3>
            <div className="mt-2 space-y-1.5">
              {instructions && instructions.sources.length > 0 ? (
                instructions.sources.map((source) => (
                  <div key={source.path} className="rounded-xl border border-border-subtle p-2">
                    <span className="flex items-center gap-1.5 text-xs text-ink-secondary">
                      {source.truncated && <AlertTriangle className="size-3 text-warning" />}
                      {source.scope} · {source.origin} · precedence {source.precedence}
                    </span>
                    <span className="mt-1 block truncate font-mono text-xs text-ink-faint">
                      {source.path}
                    </span>
                  </div>
                ))
              ) : (
                <p className="text-xs text-ink-faint">No instruction files discovered.</p>
              )}
            </div>

            {props.catalog && props.catalog.diagnostics.length > 0 && (
              <>
                <h3 className="mt-5 text-xs font-semibold uppercase tracking-wide text-ink-faint">
                  Catalog health
                </h3>
                <div className="mt-2 space-y-1.5">
                  {props.catalog.diagnostics.map((diagnostic, index) => (
                    <div
                      key={`${diagnostic.code}:${diagnostic.source}:${index}`}
                      className={cn(
                        "rounded-xl p-2 text-xs text-ink-secondary",
                        diagnostic.severity === "error" ? "bg-danger/10" : "bg-warning/10",
                      )}
                    >
                      <span
                        className={cn(
                          "font-medium",
                          diagnostic.severity === "error" ? "text-danger" : "text-warning",
                        )}
                      >
                        {diagnostic.severity} · {diagnostic.code}
                      </span>
                      <span className="mt-0.5 block">{diagnostic.message}</span>
                    </div>
                  ))}
                </div>
              </>
            )}
          </aside>
        </div>
      </div>
    </div>
  );
}
