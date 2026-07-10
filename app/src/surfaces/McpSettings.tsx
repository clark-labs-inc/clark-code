import { useEffect, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Blocks, Plus, Trash2, X, Loader2, CheckCircle2, AlertCircle, DownloadCloud } from "lucide-react";
import { useSessionStore } from "../store/sessionStore";
import {
  loadMcpServers,
  saveMcpServers,
  enabledMcpConfigs,
  blankServer,
  parseArgs,
  parseEnv,
  envToText,
  MCP_PRESETS,
  type McpServer,
} from "../lib/mcpServers";
import { probeMcp, discoverClaude, type McpStatus } from "../lib/mcp";
import { cn } from "../lib/cn";

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
        className="min-h-8 shrink-0 rounded-lg bg-accent px-2.5 py-1 text-xs font-medium text-on-accent transition duration-200 ease-clark hover:bg-accent-hover"
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
        <p className="text-xs font-medium uppercase tracking-wide text-ink-faint">Add a server</p>
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
  const activeRemote = useSessionStore((s) => s.activeRemote);
  // For a remote project, read its remote `.claude` over the tunnel.
  const cwd = activeRemote?.cwd ?? localCwd;
  const [servers, setServers] = useState<McpServer[]>([]);
  const [statuses, setStatuses] = useState<Record<string, McpStatus>>({});
  const [testing, setTesting] = useState(false);
  const [importing, setImporting] = useState(false);
  const [importNote, setImportNote] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setServers(loadMcpServers());
      setImportNote(null);
    }
  }, [open]);

  const persist = (next: McpServer[]) => {
    setServers(next);
    saveMcpServers(next);
  };
  const update = (id: string, s: McpServer) => persist(servers.map((x) => (x.id === id ? s : x)));
  const remove = (id: string) => persist(servers.filter((x) => x.id !== id));
  const add = () => persist([...servers, blankServer()]);
  const addPreset = (make: (cwd: string) => McpServer) => persist([...servers, make(cwd)]);
  const enabledCount = servers.filter((s) => s.enabled && s.command.trim()).length;

  const test = async () => {
    setTesting(true);
    try {
      const results = await probeMcp(enabledMcpConfigs(servers));
      setStatuses(Object.fromEntries(results.map((r) => [r.server, r])));
    } catch {
      /* probe is best-effort */
    } finally {
      setTesting(false);
    }
  };

  // One-click migration: pull the MCP servers (and detect skills) from an
  // existing Claude Code setup in this project. New servers merge in by name.
  const importFromClaude = async () => {
    setImporting(true);
    setImportNote(null);
    try {
      const remote = activeRemote
        ? { ws_url: activeRemote.ws_url, token: activeRemote.token }
        : undefined;
      const { mcp, skills } = await discoverClaude(cwd, remote);
      const have = new Set(servers.map((s) => s.name.trim()));
      const added = mcp
        .filter((m) => m.name.trim() && !have.has(m.name.trim()))
        .map((m) => ({
          id: crypto.randomUUID(),
          name: m.name,
          command: m.command,
          args: m.args ?? [],
          env: m.env ?? {},
          enabled: true,
        }));
      if (added.length) persist([...servers, ...added]);
      const parts: string[] = [];
      parts.push(
        added.length
          ? `Imported ${added.length} MCP server${added.length === 1 ? "" : "s"}`
          : mcp.length
            ? "MCP servers already imported"
            : "No MCP servers found",
      );
      if (skills.length)
        parts.push(`${skills.length} Claude skill${skills.length === 1 ? "" : "s"} detected — used automatically`);
      setImportNote(parts.join(" · "));
    } catch (e) {
      setImportNote(`Couldn't read Claude Code config: ${String(e)}`);
    } finally {
      setImporting(false);
    }
  };

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={reduce ? false : { opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: reduce ? 0 : 0.15 }}
          className="fixed inset-0 z-50 grid place-items-center bg-black/40 p-6"
          onClick={() => setOpen(false)}
        >
          <motion.div
            initial={reduce ? false : { opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: reduce ? 0 : 0.12 }}
            onClick={(e) => e.stopPropagation()}
            className="popover-surface flex max-h-[80vh] w-full max-w-2xl flex-col rounded-2xl border border-border bg-bg-elevated shadow-2xl"
          >
            <div className="flex items-center gap-2 border-b border-border-subtle px-4 py-3">
              <Blocks className="size-4 text-ink-secondary" />
              <h2 className="text-sm font-semibold text-ink">MCP servers</h2>
              <span className="text-xs text-ink-muted">
                Extend Clark Code with external tools
              </span>
              <button
                onClick={() => setOpen(false)}
                aria-label="Close"
                className="ml-auto grid size-7 place-items-center rounded-md text-ink-muted transition hover:bg-bg-hover hover:text-ink"
              >
                <X className="size-4" />
              </button>
            </div>

            <div className="min-h-0 flex-1 space-y-3 overflow-y-auto p-4">
              {/* Migrate an existing Claude Code setup with one click. */}
              <div className="flex items-center gap-3 rounded-xl border border-border-subtle bg-bg-elevated/40 p-3">
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium text-ink">Already use Claude Code?</p>
                  <p className="text-xs text-ink-muted">
                    Import its MCP servers from this project — skills in{" "}
                    <span className="font-mono text-ink-faint">.claude</span> are picked up
                    automatically.
                  </p>
                  {importNote && <p className="mt-1 text-xs text-ink-secondary">{importNote}</p>}
                </div>
                <button
                  onClick={() => void importFromClaude()}
                  disabled={importing || !cwd.trim()}
                  title={cwd.trim() ? "Read .mcp.json / ~/.claude.json / .claude" : "Open a project first"}
                  className="flex shrink-0 items-center gap-1.5 rounded-lg bg-bg-tertiary px-3 py-1.5 text-sm font-medium text-ink-secondary transition hover:bg-bg-hover disabled:opacity-50"
                >
                  {importing ? (
                    <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" />
                  ) : (
                    <DownloadCloud className="size-3.5" />
                  )}
                  Import from Claude Code
                </button>
              </div>

              {servers.length === 0 && (
                <p className="px-1 pb-1 text-sm text-ink-muted">
                  Add an MCP server to give Clark Code new tools. They appear alongside the
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
                  ? `${enabledCount} enabled · ${servers.length} total`
                  : "Configured per app · spawned when a session starts"}
              </span>
              <button
                onClick={() => void test()}
                disabled={testing || enabledCount === 0}
                title={enabledCount === 0 ? "Add and enable a server first" : "Connect each server and list its tools"}
                className="ml-auto flex min-h-8 items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-semibold text-on-accent transition duration-200 ease-clark hover:bg-accent-hover disabled:bg-bg-tertiary disabled:text-ink-muted"
              >
                {testing && <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" />}
                Test connections
              </button>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
