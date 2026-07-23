import { describe, it, expect } from "vitest";
import { humanizeError, humanizeRunFailure } from "./errors";

describe("humanizeRunFailure", () => {
  it("keeps session expiry and platform-key rejection distinct", () => {
    expect(humanizeRunFailure({ failure_kind: "session_expired" })).toMatch(/sign-in expired/i);
    expect(humanizeRunFailure({ failure_kind: "platform_key_rejected" })).toMatch(/access key/i);
  });

  it("trusts the typed category instead of misleading provider prose", () => {
    const msg = humanizeRunFailure({
      failure_kind: "provider_error",
      error: "403 authentication failed upstream",
    });
    expect(msg).toMatch(/provider/i);
    expect(msg).not.toMatch(/sign-in|access key/i);
  });

  it("does not infer auth from an untyped legacy error", () => {
    expect(
      humanizeRunFailure({ error: "model endpoint returned 401 Unauthorized" }),
    ).not.toMatch(/sign-in|access key/i);
  });

  it("distinguishes incomplete post-answer verification from an empty response", () => {
    const msg = humanizeRunFailure({ failure_kind: "verification_incomplete" });
    expect(msg).toMatch(/finished its answer/i);
    expect(msg).toMatch(/verify/i);
    expect(msg).not.toMatch(/no response/i);
  });
});

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
