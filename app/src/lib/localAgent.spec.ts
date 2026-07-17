import { describe, expect, it, beforeEach } from "vitest";
import {
  CODING_MODELS,
  DEFAULT_LOCAL_SETTINGS,
  REASONING_EFFORTS,
  modelLabel,
  effectiveModelSettings,
  loadChatModels,
  saveChatModels,
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
  it("keeps GLM 5.2 as the default", () => {
    expect(DEFAULT_LOCAL_SETTINGS.model).toBe("clark-code");
    expect(modelLabel(DEFAULT_LOCAL_SETTINGS.model)).toBe("GLM 5.2");
  });

  it("exposes Grok 4.5 and DeepSeek V4 Pro through backend-owned aliases", () => {
    expect(CODING_MODELS.map(({ id, label }) => ({ id, label }))).toEqual([
      { id: "clark-code", label: "GLM 5.2" },
      { id: "clark-code:kimi_k3", label: "Kimi K3" },
      { id: "clark-code:kimi_k27_code", label: "Kimi K2.7 Code" },
      { id: "clark-code:grok45", label: "Grok 4.5" },
      { id: "clark-code:deepseek_v4_pro", label: "DeepSeek V4 Pro" },
    ]);
  });

  it("offers only the portable High and Max reasoning overrides", () => {
    expect(REASONING_EFFORTS.map(({ id }) => id)).toEqual(["", "high", "xhigh"]);
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
      "chat-1": { model: "clark-code:grok45", reasoningEffort: "high" },
    };
    const out = effectiveModelSettings(DEFAULT_LOCAL_SETTINGS, overrides, "chat-1");
    expect(out.model).toBe("clark-code:grok45");
    expect(out.reasoningEffort).toBe("high");
  });

  it("one chat's override does not leak into another", () => {
    const overrides: Record<string, ChatModelOverride> = {
      "chat-1": { model: "clark-code:grok45", reasoningEffort: "" },
    };
    expect(effectiveModelSettings(DEFAULT_LOCAL_SETTINGS, overrides, "chat-2").model)
      .toBe(DEFAULT_LOCAL_SETTINGS.model);
  });
});

describe("chat model overrides round-trip localStorage", () => {
  it("persists and reloads per-chat models", () => {
    saveChatModels({ "chat-a": { model: "clark-code:grok45", reasoningEffort: "high" } });
    expect(loadChatModels()).toEqual({
      "chat-a": { model: "clark-code:grok45", reasoningEffort: "high" },
    });
    saveChatModels({});
    expect(loadChatModels()).toEqual({});
  });
});
