import { describe, expect, it, beforeEach } from "vitest";
import {
  CODING_MODELS,
  DEFAULT_LOCAL_SETTINGS,
  SPECIALIST_MODEL_ID,
  SPECIALIST_MODEL_LABEL,
  SPECIALIST_REASONING_EFFORT,
  normalizeReasoningEffort,
  modelLabel,
  effectiveModelSettings,
  addRecentProject,
  loadLocalSettings,
  loadRecentProjects,
  loadChatModels,
  loadOrchestrationEnabled,
  localConnectConfig,
  saveOrchestrationEnabled,
  saveChatModels,
  saveLocalSettings,
  type ChatModelOverride,
} from "./localAgent";

// The Node test env has no localStorage; back it with a tiny in-memory mock.
class MemStorage {
  private m = new Map<string, string>();
  get length() {
    return this.m.size;
  }
  key(i: number) {
    return [...this.m.keys()][i] ?? null;
  }
  getItem(k: string) {
    return this.m.has(k) ? this.m.get(k)! : null;
  }
  setItem(k: string, v: string) {
    this.m.set(k, String(v));
  }
  removeItem(k: string) {
    this.m.delete(k);
  }
  clear() {
    this.m.clear();
  }
}

let store: MemStorage;
beforeEach(() => {
  store = new MemStorage();
  (globalThis as { localStorage: Storage }).localStorage = store as unknown as Storage;
});

describe("product-supplied model settings", () => {
  it("uses the neutral product model by default", () => {
    expect(DEFAULT_LOCAL_SETTINGS.model).toBe("local-model");
    expect(modelLabel(DEFAULT_LOCAL_SETTINGS.model)).toBe("Local model");
  });

  it("exposes the neutral host model choices without a downstream product", () => {
    expect(CODING_MODELS.map(({ id, label }) => ({ id, label }))).toEqual([
      { id: "local-model", label: "Local model" },
      { id: "local-model-large", label: "Large local model" },
    ]);
    expect(normalizeReasoningEffort("local-model", "low")).toBe("high");
  });

  it("retires saved removed picker selections to the default model", () => {
    saveLocalSettings({
      ...DEFAULT_LOCAL_SETTINGS,
      model: "retired-model",
      reasoningEffort: "high",
    });
    saveChatModels({
      "chat-retired": { model: "retired-model", reasoningEffort: "" },
    });

    expect(loadLocalSettings()).toMatchObject({
      model: "local-model",
      reasoningEffort: "high",
    });
    expect(loadChatModels()).toEqual({
      "chat-retired": {
        model: "local-model",
        reasoningEffort: "high",
      },
    });
  });

});

describe("product specialist extension binding", () => {
  it("passes the conversation recipe to native product capability composition", () => {
    const config = localConnectConfig(
      { ...DEFAULT_LOCAL_SETTINGS, cwd: "/managed/spec-document" },
      undefined,
      undefined,
      "spec",
      "id:account",
    );

    expect(config.extra).toMatchObject({ specialist_kind: "spec" });
  });

  it("exposes account-scoped recent checkouts to Scout as read-only census roots", () => {
    const config = localConnectConfig(
      { ...DEFAULT_LOCAL_SETTINGS, cwd: "/managed/scout-conversation" },
      undefined,
      {
        organizationId: "018f8e8a-4722-7c68-b5b7-a4c6793c85b0",
        workspaceId: "028f8e8a-4722-7c68-b5b7-a4c6793c85b0",
      },
      "scout",
      "id:account",
      undefined,
      undefined,
      ["/repos/payments", "/repos/identity", "/repos/payments", ""],
    );

    expect(config.extra?.sandbox_read_roots).toEqual([
      "/repos/payments",
      "/repos/identity",
    ]);
    expect(config.cwd).toBe("/managed/scout-conversation");
  });

  it("keeps product provider extras empty in the neutral composition", () => {
    const config = localConnectConfig(
      { ...DEFAULT_LOCAL_SETTINGS, cwd: "/project" },
      undefined,
      undefined,
      "scout",
      "id:account",
      {
        organizationId: "018f8e8a-4722-7c68-b5b7-a4c6793c85b0",
        kind: "scout",
        workflow: "scout:map",
      },
    );
    expect(config.extra).toMatchObject({ model: "local-model" });
  });

  it("sends an opaque worker binding plus a credential-free specialist recipe", () => {
    const config = localConnectConfig(
      { ...DEFAULT_LOCAL_SETTINGS, cwd: "/local" },
      { worker_handle: "worker-remote", cwd: "/remote/project" },
      undefined,
      "security",
      "id:account",
      {
        organizationId: "018f8e8a-4722-7c68-b5b7-a4c6793c85b0",
        kind: "security",
        workflow: "security:scan",
        trainingOptIn: true,
      },
    );
    expect(config.cwd).toBeUndefined();
    expect(config.extra).toEqual({
      remote_worker: { worker_handle: "worker-remote", cwd: "/remote/project" },
      specialist_kind: "security",
    });
  });
});

describe("account-scoped project context", () => {
  it("does not expose another account's recent projects", () => {
    expect(loadRecentProjects("id:new-account")).toEqual([]);
    expect(addRecentProject("/new/account", "id:new-account")).toEqual(["/new/account"]);
    expect(loadRecentProjects("id:new-account")).toEqual(["/new/account"]);
    expect(loadRecentProjects("id:previous-account")).toEqual([]);
  });

  it("restores only the owning account's cwd", () => {
    saveLocalSettings(
      { ...DEFAULT_LOCAL_SETTINGS, cwd: "/previous/account" },
      "id:previous-account",
    );
    expect(loadLocalSettings("id:new-account").cwd).toBe("");
    expect(loadLocalSettings("id:previous-account").cwd).toBe("/previous/account");

    saveLocalSettings({ ...DEFAULT_LOCAL_SETTINGS, cwd: "/new/account" }, "id:new-account");
    expect(loadLocalSettings("id:new-account").cwd).toBe("/new/account");
    expect(loadLocalSettings("id:previous-account").cwd).toBe("/previous/account");
  });
});

describe("effectiveModelSettings (per-conversation model)", () => {
  it("returns the global default when no override exists", () => {
    const out = effectiveModelSettings(
      DEFAULT_LOCAL_SETTINGS,
      {},
      "chat-1",
    );
    expect(out.model).toBe(DEFAULT_LOCAL_SETTINGS.model);
    expect(out.reasoningEffort).toBe(DEFAULT_LOCAL_SETTINGS.reasoningEffort);
  });

  it("returns the global default for the start screen (no chat id)", () => {
    const out = effectiveModelSettings(DEFAULT_LOCAL_SETTINGS, {}, null);
    expect(out.model).toBe(DEFAULT_LOCAL_SETTINGS.model);
  });

  it("uses the per-chat override when one is set", () => {
    const overrides: Record<string, ChatModelOverride> = {
      "chat-1": { model: "local-model-large", reasoningEffort: "" },
    };
    const out = effectiveModelSettings(DEFAULT_LOCAL_SETTINGS, overrides, "chat-1");
    expect(out.model).toBe("local-model-large");
    expect(out.reasoningEffort).toBe("max");
  });

  it("falls back when an un-migrated chat still names a retired picker model", () => {
    const overrides: Record<string, ChatModelOverride> = {
      "chat-1": { model: "retired-model", reasoningEffort: "low" },
    };

    expect(effectiveModelSettings(DEFAULT_LOCAL_SETTINGS, overrides, "chat-1"))
      .toMatchObject({
        model: DEFAULT_LOCAL_SETTINGS.model,
        reasoningEffort: DEFAULT_LOCAL_SETTINGS.reasoningEffort,
      });
  });

  it("one chat's override does not leak into another", () => {
    const overrides: Record<string, ChatModelOverride> = {
      "chat-1": { model: "local-model-large", reasoningEffort: "" },
    };
    expect(effectiveModelSettings(DEFAULT_LOCAL_SETTINGS, overrides, "chat-2").model)
      .toBe(DEFAULT_LOCAL_SETTINGS.model);
  });
});

describe("chat model overrides round-trip localStorage", () => {
  it("persists and reloads per-chat models", () => {
    saveChatModels({ "chat-a": { model: "local-model-large", reasoningEffort: "" } });
    expect(loadChatModels()).toEqual({
      "chat-a": { model: "local-model-large", reasoningEffort: "max" },
    });
    saveChatModels({});
    expect(loadChatModels()).toEqual({});
  });
});

describe("bounded orchestration availability", () => {
  it("is enabled by default, can be disabled, and never expands a native remote binding", () => {
    expect(loadOrchestrationEnabled()).toBe(true);
    expect(localConnectConfig({ ...DEFAULT_LOCAL_SETTINGS, cwd: "/repo" }).extra)
      .toMatchObject({ orchestration: { enabled: true } });

    saveOrchestrationEnabled(false);
    expect(loadOrchestrationEnabled()).toBe(false);
    expect(localConnectConfig({ ...DEFAULT_LOCAL_SETTINGS, cwd: "/repo" }).extra)
      .toMatchObject({ orchestration: { enabled: false } });

    saveOrchestrationEnabled(true);
    expect(localConnectConfig(
      { ...DEFAULT_LOCAL_SETTINGS, cwd: "/repo" },
      { worker_handle: "worker-remote", cwd: "/remote/repo" },
    ).extra).toEqual({
      remote_worker: { worker_handle: "worker-remote", cwd: "/remote/repo" },
    });
  });
});

describe("specialist model contract", () => {
  it("keeps the specialist route available to internal workflow switching", () => {
    expect(modelLabel(SPECIALIST_MODEL_ID)).toBe(SPECIALIST_MODEL_LABEL);
    expect(normalizeReasoningEffort(SPECIALIST_MODEL_ID, "low")).toBe(SPECIALIST_REASONING_EFFORT);
    expect(effectiveModelSettings(
      { ...DEFAULT_LOCAL_SETTINGS, model: SPECIALIST_MODEL_ID, reasoningEffort: "low" },
      {},
      null,
    ).model).toBe(SPECIALIST_MODEL_ID);
  });

  it("pins every specialist connect config to the product specialist policy", () => {
    const config = localConnectConfig(
      { ...DEFAULT_LOCAL_SETTINGS, cwd: "/repo", model: "retired-specialist-model", reasoningEffort: "low" },
      undefined,
      undefined,
      "security",
    );
    expect(config.extra).toMatchObject({
      model: SPECIALIST_MODEL_ID,
      reasoning_effort: SPECIALIST_REASONING_EFFORT,
    });
  });
});

describe("retired picker model routing", () => {
  it("never sends a retired model tier to the provider", () => {
    expect(localConnectConfig({
      ...DEFAULT_LOCAL_SETTINGS,
      cwd: "/repo",
      model: "retired-model",
      reasoningEffort: "low",
    }).extra).toMatchObject({ model: "local-model" });
  });
});

describe("computer use opt-in", () => {
  it("is disabled by default and reaches the provider only after opt-in", () => {
    expect(DEFAULT_LOCAL_SETTINGS.computerUseEnabled).toBe(false);
    expect(localConnectConfig({ ...DEFAULT_LOCAL_SETTINGS, cwd: "/repo" }).extra)
      .toMatchObject({ computer_use_enabled: false });
    expect(localConnectConfig({
      ...DEFAULT_LOCAL_SETTINGS,
      cwd: "/repo",
      computerUseEnabled: true,
    }).extra).toMatchObject({ computer_use_enabled: true });
  });

});
