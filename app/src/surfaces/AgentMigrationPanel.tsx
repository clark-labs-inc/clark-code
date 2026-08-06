import { useEffect, useMemo, useState } from "react";
import { Check, DownloadCloud, Loader2 } from "lucide-react";
import {
  discoverAgentSetups,
  type AgentMigrationDiscovery,
  type MigrationSource,
} from "../lib/mcp";
import type { McpServer, McpServerConfig } from "../lib/mcpServers";

const SOURCE_LABEL: Record<MigrationSource, string> = {
  claude: "Claude Code",
  openai: "OpenAI coding agent",
};

export function AgentMigrationPanel({
  cwd,
  remote,
  servers,
  onImport,
}: {
  cwd: string;
  remote?: { id: string };
  servers: McpServer[];
  onImport: (servers: McpServerConfig[]) => number;
}) {
  const [discoveries, setDiscoveries] = useState<AgentMigrationDiscovery[] | null>(null);
  const [selected, setSelected] = useState<Record<string, string[]>>({});
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const existingNames = useMemo(
    () => new Set(servers.map((server) => server.name.trim()).filter(Boolean)),
    [servers],
  );

  useEffect(() => {
    let cancelled = false;
    setDiscoveries(null);
    setError(null);
    void discoverAgentSetups(cwd, remote)
      .then((found) => {
        if (cancelled) return;
        setDiscoveries(found);
        setSelected(
          Object.fromEntries(
            found.map((source) => [
              source.source,
              source.mcp
                .map((server) => server.name)
                .filter((name) => !existingNames.has(name.trim())),
            ]),
          ),
        );
      })
      .catch((reason) => {
        if (!cancelled) {
          setDiscoveries([]);
          setError(reason instanceof Error ? reason.message : String(reason));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [cwd, remote?.id]);

  if (!cwd.trim()) return null;
  if (discoveries === null) {
    return (
      <div className="flex items-center gap-2 px-1 py-2 text-xs text-ink-muted">
        <Loader2 className="size-3.5 animate-[spin_1s_linear_infinite]" />
        Checking for compatible coding-agent setup…
      </div>
    );
  }
  if (discoveries.length === 0 && !error) return null;

  const toggle = (source: MigrationSource, name: string) => {
    setSelected((current) => {
      const names = current[source] ?? [];
      return {
        ...current,
        [source]: names.includes(name)
          ? names.filter((candidate) => candidate !== name)
          : [...names, name],
      };
    });
  };

  return (
    <section className="space-y-2 rounded-xl border border-border-subtle bg-bg-elevated/40 p-3">
      <div>
        <p className="text-sm font-medium text-ink">Bring over another coding agent</p>
        <p className="text-xs text-ink-muted">
          Review detected setup before importing. Clark adds only missing MCP servers; source files
          stay unchanged.
        </p>
      </div>
      {error && <p className="text-xs text-danger">Couldn&apos;t inspect agent setup: {error}</p>}
      {discoveries.map((discovery) => {
        const chosen = new Set(selected[discovery.source] ?? []);
        const available = discovery.mcp.filter((server) => !existingNames.has(server.name.trim()));
        const label = SOURCE_LABEL[discovery.source];
        return (
          <div key={discovery.source} className="rounded-lg border border-border-subtle bg-bg p-2.5">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <p className="text-sm font-medium text-ink">{label} detected</p>
                <p className="text-xs text-ink-muted">
                  {discovery.mcp.length} MCP · {discovery.skills.length} skill
                  {discovery.skills.length === 1 ? "" : "s"} · {discovery.instructions.length}{" "}
                  instruction file{discovery.instructions.length === 1 ? "" : "s"}
                </p>
              </div>
              {available.length > 0 ? (
                <button
                  type="button"
                  disabled={chosen.size === 0}
                  onClick={() => {
                    const imported = onImport(
                      discovery.mcp.filter((server) => chosen.has(server.name)),
                    );
                    setSelected((current) => ({ ...current, [discovery.source]: [] }));
                    setNotes((current) => ({
                      ...current,
                      [discovery.source]: `Imported ${imported} MCP server${imported === 1 ? "" : "s"}.`,
                    }));
                  }}
                  className="flex shrink-0 items-center gap-1.5 rounded-lg bg-bg-tertiary px-2.5 py-1.5 text-xs font-medium text-ink-secondary transition hover:bg-bg-hover disabled:opacity-50"
                >
                  <DownloadCloud className="size-3.5" /> Import selected
                </button>
              ) : (
                <span className="flex items-center gap-1 text-xs text-success">
                  <Check className="size-3.5" /> Compatible setup active
                </span>
              )}
            </div>
            {discovery.mcp.length > 0 && (
              <div className="mt-2 grid gap-1 sm:grid-cols-2">
                {discovery.mcp.map((server) => {
                  const imported = existingNames.has(server.name.trim());
                  return (
                    <label
                      key={server.name}
                      className="flex min-w-0 items-center gap-2 rounded-md px-1.5 py-1 text-xs text-ink-secondary"
                    >
                      <input
                        type="checkbox"
                        checked={imported || chosen.has(server.name)}
                        disabled={imported}
                        onChange={() => toggle(discovery.source, server.name)}
                        className="accent-accent"
                      />
                      <span className="truncate font-mono">{server.name}</span>
                      {imported && <span className="ml-auto text-ink-faint">Already added</span>}
                    </label>
                  );
                })}
              </div>
            )}
            {(discovery.skills.length > 0 || discovery.instructions.length > 0) && (
              <p className="mt-1.5 text-xs text-ink-muted">
                Skills and instructions stay in place and are available automatically in new chats.
              </p>
            )}
            {notes[discovery.source] && (
              <p className="mt-1.5 text-xs text-success">{notes[discovery.source]}</p>
            )}
          </div>
        );
      })}
    </section>
  );
}
