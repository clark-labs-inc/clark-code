import { beforeEach, describe, expect, it } from "vitest";
import { accountScopedKey } from "./accountProjectStorage";
import { allowCommand, denyCommand, loadAllowlist, loadDenylist } from "./commandPolicy";
import {
  DEFAULT_LOCAL_SETTINGS,
  loadBrowserEnabled,
  loadChatModels,
  loadLocalSettings,
  loadMemoriesEnabled,
  loadOrchestrationEnabled,
  localConnectConfig,
  saveBrowserEnabled,
  saveChatModels,
  saveLocalSettings,
  saveMemoriesEnabled,
  saveOrchestrationEnabled,
} from "./localAgent";
import { loadMcpServers, saveMcpServers, type McpServer } from "./mcpServers";
import { organizationForRepository, setOrganizationForRepository } from "./organizationKnowledge";
import { loadOutputStyle, saveOutputStyle } from "./outputStyle";
import {
  loadApprovalPolicy,
  loadCollaborationMode,
  loadCollaborationModes,
  saveApprovalPolicy,
  saveCollaborationMode,
  saveCollaborationModes,
} from "./permissions";
import { selectedSecurityOrganization, selectSecurityOrganization } from "./securityCloud";
import { loadSshHosts, saveSshHosts, type SshHost } from "./sshHosts";

const accountOne = "id:account-one";
const accountTwo = "id:account-two";

beforeEach(() => localStorage.clear());

describe("account-owned desktop state", () => {
  it("uses separate storage namespaces for authenticated and signed-out state", () => {
    expect(accountScopedKey("setting", accountOne)).not.toBe(
      accountScopedKey("setting", accountTwo),
    );
    expect(accountScopedKey("setting", null)).not.toBe(
      accountScopedKey("setting", accountOne),
    );
  });

  it("does not expose models or feature choices to another account", () => {
    saveLocalSettings({
      ...DEFAULT_LOCAL_SETTINGS,
      cwd: "/account-one/project",
      model: "local-model-large",
      reasoningEffort: "max",
      computerUseEnabled: true,
    }, accountOne);
    saveChatModels({
      "shared-conversation-id": {
        model: "local-model-large",
        reasoningEffort: "max",
      },
    }, accountOne);
    saveMemoriesEnabled(false, accountOne);
    saveBrowserEnabled(true, accountOne);
    saveOrchestrationEnabled(false, accountOne);

    expect(loadLocalSettings(accountTwo)).toEqual(DEFAULT_LOCAL_SETTINGS);
    expect(loadChatModels(accountTwo)).toEqual({});
    expect(loadMemoriesEnabled(accountTwo)).toBe(true);
    expect(loadBrowserEnabled(accountTwo)).toBe(false);
    expect(loadOrchestrationEnabled(accountTwo)).toBe(true);
  });

  it("does not expose MCP environment secrets, SSH targets, or command trust", () => {
    const server: McpServer = {
      id: "private-mcp",
      name: "private",
      command: "private-mcp",
      args: [],
      env: { PRIVATE_TOKEN: "account-one-secret" },
      enabled: true,
    };
    const host: SshHost = {
      id: "private-host",
      label: "Account one GPU",
      host: "account-one-gpu",
      remoteRoot: "/workspace/private",
    };
    saveMcpServers([server], accountOne);
    saveSshHosts([host], accountOne);
    allowCommand("/shared/project", "deploy account one", accountOne);
    denyCommand("/shared/project", "erase account one", accountOne);

    expect(loadMcpServers(accountTwo)).toEqual([]);
    expect(loadSshHosts(accountTwo)).toEqual([]);
    expect(loadAllowlist("/shared/project", accountTwo)).toEqual([]);
    expect(loadDenylist("/shared/project", accountTwo)).toEqual([]);

    const config = localConnectConfig(
      { ...DEFAULT_LOCAL_SETTINGS, cwd: "/shared/project" },
      undefined,
      undefined,
      undefined,
      accountTwo,
    );
    expect(config.extra).toMatchObject({
      command_allowlist: [],
      command_denylist: [],
      mcp_servers: [],
    });
    expect(config.extra).not.toHaveProperty("memory_scope");
  });

  it("does not reuse execution modes, output style, or organization choices", () => {
    saveApprovalPolicy("full", accountOne);
    saveCollaborationMode("plan", accountOne);
    saveCollaborationModes({ "shared-conversation-id": "plan" }, accountOne);
    saveOutputStyle("teaching", accountOne);
    setOrganizationForRepository("repo-fingerprint", "org-one", accountOne);
    selectSecurityOrganization("repo-fingerprint", "security-org-one", accountOne);

    expect(loadApprovalPolicy(accountTwo)).toBe("auto");
    expect(loadCollaborationMode(accountTwo)).toBe("default");
    expect(loadCollaborationModes(accountTwo)).toEqual({});
    expect(loadOutputStyle(accountTwo)).toBe("default");
    expect(organizationForRepository("repo-fingerprint", accountTwo)).toBeNull();
    expect(selectedSecurityOrganization("repo-fingerprint", accountTwo)).toBeNull();
  });
});
