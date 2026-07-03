import { describe, it, expect } from "vitest";
import { humanizeError } from "./errors";

describe("humanizeError", () => {
  it("collapses the raw 429 provider JSON blob into one friendly line", () => {
    const raw =
      'model endpoint returned 429 Too Many Requests: {"error":{"message":"Provider returned error","code":429,"metadata":{"raw":"moonshotai/kimi-k2.7-code is temporarily rate-limited upstream. Please retry shortly, or add your own key to accumulate your rate limits: https://openrouter.ai/settings/integrations","provider_name":"DeepInfra","is_byok":false}},"user_id":"user_3A8lOkOB3knNQ5GzXvi9p8X0aBk"}';
    const msg = humanizeError(raw);
    expect(msg).toBe("The model is busy right now (rate-limited). Give it a moment and try again.");
    expect(msg).not.toContain("{");
    expect(msg).not.toContain("user_id");
    expect(msg.length).toBeLessThan(120);
  });

  it("recognizes auth failures", () => {
    expect(humanizeError("model endpoint returned 401 Unauthorized: ...")).toMatch(/sign/i);
  });

  it("recognizes context-window overflow", () => {
    expect(humanizeError("400: maximum context length is 200000 tokens")).toMatch(/context/i);
  });

  it("recognizes network/timeout errors", () => {
    expect(humanizeError("model request failed: error sending request (connection reset)")).toMatch(
      /connection|reach/i,
    );
  });

  it("maps 5xx / provider errors to a retry message", () => {
    expect(humanizeError("model endpoint returned 503 Service Unavailable: upstream")).toMatch(
      /try again/i,
    );
  });

  it("handles empty/nullish input gracefully", () => {
    expect(humanizeError("")).toBeTruthy();
    expect(humanizeError(null)).toBeTruthy();
    expect(humanizeError(undefined)).toBeTruthy();
  });

  it("cleans an unknown JSON error down to its message field without dumping JSON", () => {
    const raw = 'model endpoint returned 400 Bad Request: {"error":{"message":"model `foo` not found"}}';
    const msg = humanizeError(raw);
    expect(msg).toContain("not found");
    expect(msg).not.toContain("{");
  });

  it("truncates a very long unknown error", () => {
    const msg = humanizeError("weird failure ".repeat(50));
    expect(msg.length).toBeLessThanOrEqual(160);
  });
});
