import { useEffect, useState } from "react";
import { productName } from "../product/productModule";
import { AnimatePresence, useReducedMotion } from "motion/react";
import * as m from "motion/react-m";
import { Blocks, Plus, Trash2, X, Loader2, CheckCircle2, AlertCircle } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import {
  loadMcpServers,
  saveMcpServers,
  enabledMcpConfigs,
  redactMcpSecrets,
  blankServer,
  parseArgs,
  parseEnv,
  envToText,
  MCP_PRESETS,
  type McpServer,
} from "../lib/mcpServers";
import { mergeDiscoveredMcp, probeMcp, syncMcpCredentials, type McpStatus } from "../lib/mcp";
import { cn } from "../lib/cn";
import { DIALOG, OVERLAY, accessibleMotion } from "../lib/motion";
import { AgentMigrationPanel } from "./AgentMigrationPanel";
import { codeKeyAccountBinding } from "../lib/account";

const input =
  "w-full rounded-lg border border-border bg-bg px-2.5 py-1.5 text-sm text-ink outline-none transition focus:border-accent placeholder:text-ink-muted";
const label = "mb-1 block text-xs font-medium text-ink-secondary";

function ServerCard({
  server,
  status,
  onChange,
  onRemove,
}: {
  server: McpServer;
  status?: McpStatus;
  onChange: (s: McpServer) => void;
  onRemove: () => void;
}) {
  return (
    <div className="rounded-xl border border-border-subtle bg-bg-elevated/40 p-3">
      <div className="mb-2.5 flex items-center gap-2">
        <input
          value={server.name}
          onChange={(e) => onChange({ ...server, name: e.target.value })}
          placeholder="name (e.g. github)"
          className={cn(input, "flex-1 font-medium")}
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
        />
        <button
          onClick={() => onChange({ ...server, enabled: !server.enabled })}
          className={cn(
            "rounded-md px-2 py-1 text-xs font-medium transition",
            server.enabled
              ? "bg-success/15 text-success"
              : "bg-bg-tertiary text-ink-muted hover:bg-bg-hover",
          )}
        >
          {server.enabled ? "Enabled" : "Disabled"}
        </button>
        <button
          onClick={onRemove}
          aria-label="Remove server"
          className="grid size-7 place-items-center rounded-md text-ink-muted transition hover:bg-danger/15 hover:text-danger"
        >
          <Trash2 className="size-3.5" />
        </button>
      </div>

      <div className="grid grid-cols-[1fr_1fr] gap-2">
        <div>
          <label className={label}>Command</label>
          <input
            value={server.command}
            onChange={(e) => onChange({ ...server, command: e.target.value })}
            placeholder="npx"
            className={input}
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
          />
        </div>
        <div>
          <label className={label}>Args (one per line)</label>
          <textarea
            value={server.args.join("\n")}
            onChange={(e) => onChange({ ...server, args: parseArgs(e.target.value) })}
            placeholder={"-y\n@modelcontextprotocol/server-filesystem\n."}
            rows={3}
            className={cn(input, "resize-none font-mono text-xs")}
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
          />
        </div>
      </div>

      <div className="mt-2">
        <label className={label}>Environment (KEY=value per line)</label>
        <textarea
          value={envToText(server.env)}
          onChange={(e) => onChange({ ...server, env: parseEnv(e.target.value) })}
          placeholder="GITHUB_TOKEN=ghp_…"
          rows={2}
          className={cn(input, "resize-none font-mono text-xs")}
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
        />
      </div>

      {status && (
        <div
          className={cn(
            "mt-2.5 flex items-start gap-2 rounded-lg px-2.5 py-1.5 text-xs",
            status.connected ? "bg-success/10 text-ink-secondary" : "bg-danger/10 text-ink-secondary",
          )}
        >
          {status.connected ? (
            <CheckCircle2 className="mt-0.5 size-3.5 shrink-0 text-success" />
          ) : (
            <AlertCircle className="mt-0.5 size-3.5 shrink-0 text-danger" />
          )}
          <span className="min-w-0">
            {status.connected ? (
              <>
                Connected · {status.tool_count} tool{status.tool_count === 1 ? "" : "s"}
                {status.tools.length > 0 && (
                  <span className="ml-1 font-mono text-ink-faint">{status.tools.join(", ")}</span>
                )}
              </>
            ) : (
              <span className="text-danger">{status.error ?? "Failed to connect"}</span>
            )}
          </span>
        </div>
      )}
    </div>
  );
}

function CatalogCard({ preset, onAdd }: { preset: (typeof MCP_PRESETS)[number]; onAdd: () => void }) {
  return (
    <div className="flex items-start gap-2 rounded-lg border border-border-subtle bg-bg-elevated/30 p-2.5">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-ink">{preset.label}</div>
        <div className="truncate text-xs text-ink-muted">{preset.description}</div>
        {preset.needs && <div className="mt-0.5 text-xs text-warning">needs {preset.needs}</div>}
      </div>
      <button
        onClick={onAdd}
        className="min-h-8 shrink-0 rounded-lg bg-accent px-2.5 py-1 text-xs font-medium text-on-accent transition duration-base ease-agent hover:bg-accent-hover"
      >
        Add
      </button>
    </div>
  );
}

const CATEGORIES = ["Code", "Web", "Data", "Knowledge"] as const;

function Catalog({ onAdd, addBlank }: { onAdd: (make: (cwd: string) => McpServer) => void; addBlank: () => void }) {
  return (
    <div>
      <div className="mb-2 flex items-center justify-between">
        <span className="text-xs font-semibold uppercase tracking-wider text-ink-faint">Add a server</span>
        <button
          onClick={addBlank}
          className="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-xs font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink"
        >
          <Plus className="size-3" /> Custom
        </button>
      </div>
      {CATEGORIES.map((cat) => {
        const items = MCP_PRESETS.filter((p) => p.category === cat);
        if (!items.length) return null;
        return (
          <div key={cat} className="mb-3 last:mb-0">
            <p className="mb-1.5 text-xs text-ink-faint">{cat}</p>
            <div className="grid grid-cols-2 gap-2">
              {items.map((p) => (
                <CatalogCard key={p.id} preset={p} onAdd={() => onAdd(p.make)} />
              ))}
            </div>
          </div>
        );
      })}
    </div>
  );
}

export function McpSettings() {
  const open = useSessionStore((s) => s.mcpOpen);
  const setOpen = useSessionStore((s) => s.setMcpOpen);
  // Instant, no opacity fade under Reduced Motion — see Settings for why.
  const reduce = useReducedMotion();
  const localCwd = useSessionStore((s) => s.localSettings.cwd);
  const activeProjectRoot = useSessionStore((s) => s.activeProjectRoot);
  const activeRemote = useSessionStore((s) => s.activeRemote);
  const auth = useSessionStore((s) => s.auth);
  const accountScope = codeKeyAccountBinding(auth);
  // For a remote project, inspect agent setup through its native worker handle.
  const cwd = activeRemote?.cwd ?? activeProjectRoot ?? localCwd;
  const [servers, setServers] = useState<McpServer[]>([]);
  const [savedServers, setSavedServers] = useState<McpServer[]>([]);
  const [statuses, setStatuses] = useState<Record<string, McpStatus>>({});
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      const loaded = loadMcpServers(accountScope);
      const redacted = redactMcpSecrets(loaded);
      const hasLegacySecrets = loaded.some((server) =>
        Object.values(server.env).some((value) => value.length > 0),
      );
      setServers(hasLegacySecrets ? loaded : redacted);
      setSavedServers(redacted);
      setSaveError(null);
      if (hasLegacySecrets) {
        void syncMcpCredentials(loaded)
          .then(() => {
            saveMcpServers(redacted, accountScope);
            setServers(redacted);
          })
          .catch((error: unknown) => {
            setSaveError(error instanceof Error ? error.message : String(error));
          });
      }
    }
  }, [accountScope, open]);

  const update = (id: string, s: McpServer) =>
    setServers((current) => current.map((x) => (x.id === id ? s : x)));
  const remove = (id: string) => setServers((current) => current.filter((x) => x.id !== id));
  const add = () => setServers((current) => [...current, blankServer()]);
  const addPreset = (make: (cwd: string) => McpServer) =>
    setServers((current) => [...current, make(cwd)]);
  const enabledCount = servers.filter((s) => s.enabled && s.command.trim()).length;
  const dirty = JSON.stringify(servers) !== JSON.stringify(savedServers);
  const close = () => setOpen(false);
  const save = async () => {
    setSaving(true);
    setSaveError(null);
    try {
      await syncMcpCredentials(servers);
      const redacted = redactMcpSecrets(servers);
      saveMcpServers(redacted, accountScope);
      setServers(redacted);
      setSavedServers(redacted);
      setOpen(false);
    } catch (error) {
      setSaveError(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  };

  const test = async () => {
    setTesting(true);
    try {
      await syncMcpCredentials(servers);
      const results = await probeMcp(enabledMcpConfigs(servers));
      setStatuses(Object.fromEntries(results.map((r) => [r.server, r])));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setStatuses(
        Object.fromEntries(
          enabledMcpConfigs(servers).map((server) => [
            server.name,
            { server: server.name, connected: false, tool_count: 0, tools: [], error: message },
          ]),
        ),
      );
    } finally {
      setTesting(false);
    }
  };

  return (
    <AnimatePresence>
      {open && (
        <m.div
          {...accessibleMotion(OVERLAY, reduce)}
          className="fixed inset-0 z-50 grid place-items-center bg-black/40 p-6"
          onClick={close}
        >
          <m.div
            {...accessibleMotion(DIALOG, reduce)}
            role="dialog"
            aria-modal="true"
            aria-labelledby="mcp-settings-title"
            onClick={(e) => e.stopPropagation()}
            className="popover-surface flex max-h-[80vh] w-full max-w-2xl flex-col rounded-2xl border border-border bg-bg-elevated shadow-2xl"
          >
            <div className="flex items-center gap-2 border-b border-border-subtle px-4 py-3">
              <Blocks className="size-4 text-ink-secondary" />
              <h2 id="mcp-settings-title" className="text-sm font-semibold text-ink">
                MCP servers
              </h2>
              <span className="text-xs text-ink-muted">
                Extend {productName()} with external tools
              </span>
              <button
                onClick={close}
                aria-label="Close"
                className="ml-auto grid size-7 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink"
              >
                <X className="size-4" />
              </button>
            </div>

            <div className="min-h-0 flex-1 space-y-3 overflow-y-auto p-4">
              <AgentMigrationPanel
                cwd={cwd}
                remote={
                  activeRemote
                    ? { id: activeRemote.id }
                    : undefined
                }
                servers={servers}
                onImport={(discovered) => {
                  const merged = mergeDiscoveredMcp(servers, discovered);
                  if (merged.added > 0) setServers(merged.servers);
                  return merged.added;
                }}
              />

              {servers.length === 0 && (
                <p className="px-1 pb-1 text-sm text-ink-muted">
                  Add an MCP server to give {productName()} new tools. They appear alongside the
                  built-ins and pass through the same approval gate.
                </p>
              )}
              {servers.map((s) => (
                <ServerCard
                  key={s.id}
                  server={s}
                  status={statuses[s.name.trim()]}
                  onChange={(next) => update(s.id, next)}
                  onRemove={() => remove(s.id)}
                />
              ))}
              {servers.length > 0 && <div className="border-t border-border-subtle" />}
              <Catalog onAdd={addPreset} addBlank={add} />
            </div>

            <div className="flex items-center gap-2 border-t border-border-subtle px-4 py-3">
              <span className="text-xs text-ink-faint">
                {servers.length > 0
                  ? `${enabledCount} enabled · ${servers.length} total${dirty ? " · unsaved changes" : ""}`
                  : "Configured per app · spawned when a session starts"}
              </span>
              {saveError && <span className="max-w-52 truncate text-xs text-danger">{saveError}</span>}
              <button
                onClick={() => void test()}
                disabled={testing || enabledCount === 0}
                title={enabledCount === 0 ? "Add and enable a server first" : "Connect each server and list its tools"}
                className="ml-auto flex min-h-8 items-center gap-1.5 rounded-lg bg-bg-tertiary px-3 py-1.5 text-sm font-medium text-ink-secondary transition duration-base ease-agent hover:bg-bg-hover disabled:text-ink-muted disabled:opacity-50"
              >
                {testing && <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" />}
                Test connections
              </button>
              <button
                type="button"
                onClick={close}
                className="min-h-8 rounded-lg px-3 py-1.5 text-sm font-medium text-ink-muted transition hover:bg-bg-hover hover:text-ink"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => void save()}
                disabled={saving}
                className="min-h-8 rounded-lg bg-accent px-3 py-1.5 text-sm font-semibold text-on-accent transition duration-base ease-agent hover:bg-accent-hover"
              >
                {saving ? "Saving…" : "Save"}
              </button>
            </div>
          </m.div>
        </m.div>
      )}
    </AnimatePresence>
  );
}
