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

describe("Clark Code model settings", () => {
  it("keeps the included coding route as the default", () => {
    expect(DEFAULT_LOCAL_SETTINGS.model).toBe("clark-code:free");
    expect(modelLabel(DEFAULT_LOCAL_SETTINGS.model)).toBe("Free");
  });

  it("exposes the three current Clark Code model tiers", () => {
    expect(CODING_MODELS.map(({ id, label }) => ({ id, label }))).toEqual([
      { id: "clark-code:free", label: "Free" },
      { id: "clark-code:glm52", label: "GLM 5.2" },
      { id: "clark-code:kimi_k3", label: "Kimi K3" },
    ]);
  });

  it("describes each model by its best use", () => {
    expect(CODING_MODELS.map(({ id, hint }) => ({ id, hint }))).toEqual([
      { id: "clark-code:free", hint: "Fast coding and agent work" },
      { id: "clark-code:glm52", hint: "Daily driver for coding and security" },
      { id: "clark-code:kimi_k3", hint: "Super intelligence" },
    ]);
  });

  it("pins every selectable model to its maximum reasoning effort", () => {
    expect(CODING_MODELS.map(({ id, defaultReasoningEffort }) => ({ id, defaultReasoningEffort })))
      .toEqual([
        { id: "clark-code:free", defaultReasoningEffort: "max" },
        { id: "clark-code:glm52", defaultReasoningEffort: "xhigh" },
        { id: "clark-code:kimi_k3", defaultReasoningEffort: "max" },
      ]);
    expect(normalizeReasoningEffort("clark-code:free", "low")).toBe("max");
    expect(normalizeReasoningEffort("clark-code:glm52", "")).toBe("xhigh");
    expect(normalizeReasoningEffort("clark-code:kimi_k3", "high")).toBe("max");
  });

  it("retires saved removed picker selections to the default model", () => {
    saveLocalSettings({
      ...DEFAULT_LOCAL_SETTINGS,
      model: "clark-code:grok45",
      reasoningEffort: "high",
    });
    saveChatModels({
      "chat-retired": { model: "clark-code:claude_opus_5", reasoningEffort: "" },
    });

    expect(loadLocalSettings()).toMatchObject({
      model: "clark-code:free",
      reasoningEffort: "max",
    });
    expect(loadChatModels()).toEqual({
      "chat-retired": {
        model: "clark-code:free",
        reasoningEffort: "max",
      },
    });
  });

  it("keeps the included coding route without rewriting it", () => {
    saveLocalSettings({ ...DEFAULT_LOCAL_SETTINGS, model: "clark-code:free" });

    expect(loadLocalSettings()).toMatchObject({
      model: "clark-code:free",
      reasoningEffort: "max",
    });
  });

});

describe("cloud advisor host binding", () => {
  it("binds a local Scout session without granting training eligibility", () => {
    const config = localConnectConfig(
      { ...DEFAULT_LOCAL_SETTINGS, cwd: "/project" },
      undefined,
      undefined,
      "scout",
      "id:account",
      {
        organizationId: "018f8e8a-4722-7c68-b5b7-a4c6793c85b0",
        specialist: "scout",
        workflow: "scout:map",
      },
    );
    expect(config.extra?.cloud_advisor).toEqual({
      organization_id: "018f8e8a-4722-7c68-b5b7-a4c6793c85b0",
      specialist: "scout",
      workflow: "scout:map",
      execution_residency: "local_only",
      training_consent: "none",
    });
  });

  it("sends only an opaque native binding for a remote worker", () => {
    const config = localConnectConfig(
      { ...DEFAULT_LOCAL_SETTINGS, cwd: "/local" },
      { worker_handle: "worker-remote", cwd: "/remote/project" },
      undefined,
      "security",
      "id:account",
      {
        organizationId: "018f8e8a-4722-7c68-b5b7-a4c6793c85b0",
        specialist: "security",
        workflow: "security:scan",
        trainingConsent: "explicit_user",
      },
    );
    expect(config.cwd).toBeUndefined();
    expect(config.extra).toEqual({
      remote_worker: { worker_handle: "worker-remote", cwd: "/remote/project" },
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
      "chat-1": { model: "clark-code:free", reasoningEffort: "" },
    };
    const out = effectiveModelSettings(DEFAULT_LOCAL_SETTINGS, overrides, "chat-1");
    expect(out.model).toBe("clark-code:free");
    expect(out.reasoningEffort).toBe("max");
  });

  it("falls back when an un-migrated chat still names a retired picker model", () => {
    const overrides: Record<string, ChatModelOverride> = {
      "chat-1": { model: "clark-code:gemini35_flash_lite", reasoningEffort: "low" },
    };

    expect(effectiveModelSettings(DEFAULT_LOCAL_SETTINGS, overrides, "chat-1"))
      .toMatchObject({ model: DEFAULT_LOCAL_SETTINGS.model, reasoningEffort: "max" });
  });

  it("one chat's override does not leak into another", () => {
    const overrides: Record<string, ChatModelOverride> = {
      "chat-1": { model: "clark-code:free", reasoningEffort: "" },
    };
    expect(effectiveModelSettings(DEFAULT_LOCAL_SETTINGS, overrides, "chat-2").model)
      .toBe(DEFAULT_LOCAL_SETTINGS.model);
  });
});

describe("chat model overrides round-trip localStorage", () => {
  it("persists and reloads per-chat models", () => {
    saveChatModels({ "chat-a": { model: "clark-code:free", reasoningEffort: "" } });
    expect(loadChatModels()).toEqual({
      "chat-a": { model: "clark-code:free", reasoningEffort: "max" },
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

  it("pins every specialist connect config to DeepSeek at maximum reasoning", () => {
    const config = localConnectConfig(
      { ...DEFAULT_LOCAL_SETTINGS, cwd: "/repo", model: "clark-code:deepseek_v4_flash_latest", reasoningEffort: "low" },
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
      model: "clark-code:gemini35_flash_lite",
      reasoningEffort: "low",
    }).extra).toMatchObject({ model: "clark-code:free" });
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
