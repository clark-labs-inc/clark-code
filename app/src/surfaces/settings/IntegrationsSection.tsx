import { useMemo } from "react";
import { Blocks, Server } from "lucide-react";
import { useSessionStore } from "../../store/sessionStore";
import { codeKeyAccountBinding } from "../../lib/account";
import { loadMcpServers } from "../../lib/mcpServers";
import { loadSshHosts } from "../../lib/sshHosts";
import { productName } from "../../product/productModule";
import { Card, GroupLabel, Row } from "./Primitives";
import { NativeIntegrations } from "./NativeIntegrations";

export function IntegrationsSection() {
  const setSettingsOpen = useSessionStore((s) => s.setSettingsOpen);
  const setMcpOpen = useSessionStore((s) => s.setMcpOpen);
  const setSshOpen = useSessionStore((s) => s.setSshOpen);
  const auth = useSessionStore((s) => s.auth);
  const accountScope = codeKeyAccountBinding(auth);
  const servers = useMemo(() => loadMcpServers(accountScope), [accountScope]);
  const hosts = useMemo(() => loadSshHosts(accountScope), [accountScope]);
  const mcpEnabled = servers.filter((s) => s.enabled && s.command.trim()).length;

  const manage = (open: () => void) => {
    setSettingsOpen(false);
    open();
  };

  return (
    <div className="space-y-6">
      <NativeIntegrations />
      <div>
        <GroupLabel>Extend {productName()}</GroupLabel>
        <Card>
          <Row
            icon={<Blocks className="size-4" />}
            name="MCP servers"
            sub={
              servers.length
                ? `${mcpEnabled} enabled · ${servers.length} configured`
                : "Add external tools via Model Context Protocol"
            }
          >
            <button
              onClick={() => manage(() => setMcpOpen(true))}
              className="shrink-0 rounded-lg bg-bg-tertiary px-2.5 py-1.5 text-xs font-medium text-ink-secondary transition hover:bg-bg-hover"
            >
              Manage
            </button>
          </Row>
          <Row
            icon={<Server className="size-4" />}
            name="Remote hosts"
            sub={
              hosts.length
                ? `${hosts.length} host${hosts.length === 1 ? "" : "s"} saved`
                : "Run the agent on a machine over SSH"
            }
          >
            <button
              onClick={() => manage(() => setSshOpen(true))}
              className="shrink-0 rounded-lg bg-bg-tertiary px-2.5 py-1.5 text-xs font-medium text-ink-secondary transition hover:bg-bg-hover"
            >
              Manage
            </button>
          </Row>
        </Card>
      </div>
    </div>
  );
}
