import { describe, it, expect } from "vitest";
import {
  humanizeError,
  humanizeRunFailure,
  isAccountReconnectError,
  isQuietRetryableRunFailure,
} from "./errors";
import { installProductModule, neutralProduct } from "../product/productModule";

describe("humanizeRunFailure", () => {
  it("keeps session expiry and platform-key rejection distinct", () => {
    expect(humanizeRunFailure({ failure_kind: "session_expired" })).toMatch(/sign-in expired/i);
    expect(humanizeRunFailure({ failure_kind: "platform_key_rejected" })).toMatch(/access key/i);
    expect(humanizeRunFailure({ failure_kind: "access_scope_required" })).toMatch(/active Clark workspace/i);
  });

  it("trusts the typed category instead of misleading provider prose", () => {
    const msg = humanizeRunFailure({
      failure_kind: "provider_error",
      error: "403 authentication failed upstream",
    });
    expect(msg).toMatch(/taking longer/i);
    expect(msg).not.toMatch(/provider|rate.limit|transport|error/i);
    expect(msg).not.toMatch(/sign-in|access key/i);
  });

  it("keeps transient infrastructure failures out of the danger surface", () => {
    for (const failure_kind of [
      "provider_error",
      "rate_limited",
      "transport_error",
      "empty_response",
    ] as const) {
      expect(isQuietRetryableRunFailure({ failure_kind })).toBe(true);
    }
    expect(isQuietRetryableRunFailure({ failure_kind: "tool_fatal" })).toBe(false);
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

  it("presents an iteration limit as resumable saved work", () => {
    const msg = humanizeRunFailure({ failure_kind: "iteration_limit" });

    expect(msg).toMatch(/step limit/i);
    expect(msg).toMatch(/continue in this task/i);
    expect(msg).not.toMatch(/start another run/i);
  });

  it("does not recommend retrying deterministic inconsistent tool history", () => {
    const msg = humanizeRunFailure({ failure_kind: "inconsistent_tool_history" });

    expect(msg).toMatch(/incomplete coding-step record/i);
    expect(msg).toMatch(/start a new task/i);
    expect(msg).not.toMatch(/try again|temporary/i);
  });

  it("keeps provider usage failures product-neutral", () => {
    expect(humanizeRunFailure({ failure_kind: "insufficient_credits" }))
      .toMatch(/selected provider/i);
  });
});

describe("humanizeError", () => {
  it("turns native account-authority failures into one recovery action", () => {
    installProductModule({
      ...neutralProduct,
      errors: {
        isAccountReconnectError: (raw) => raw.startsWith("example_account_mismatch:"),
      },
    });
    const current = "example_account_mismatch: another account owns this session";
    const unrelated = "provider credentials do not match";

    expect(isAccountReconnectError(current)).toBe(true);
    expect(isAccountReconnectError(unrelated)).toBe(false);
    expect(humanizeError(current)).toBe(
      "the agent needs to reconnect your account. Sign out and sign in again.",
    );
    installProductModule(neutralProduct);
  });

  it("collapses the raw 429 provider JSON blob into one friendly line", () => {
    const raw =
      'model endpoint returned 429 Too Many Requests: {"error":{"message":"Provider returned error","code":429,"metadata":{"raw":"vendor/model is temporarily rate-limited upstream. Please retry shortly.","provider_name":"ExampleProvider","is_byok":false}},"user_id":"example-user"}';
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

  it("does not expose native snapshot/serde diagnostics", () => {
    const msg = humanizeError(
      "invalid args `baseSnapshot` for command `session_configure_cloud`: unknown variant `plan`",
    );
    expect(msg).toBe("Clark Code couldn’t restore this conversation. Please try again.");
    expect(msg).not.toContain("baseSnapshot");
    expect(msg).not.toContain("plan");
  });

  it("never exposes goal lifecycle tool names from a rejected prompt", () => {
    const msg = humanizeError(
      "an unfinished goal already exists (blocked): finish it with update_goal or ask the user to clear it",
    );

    expect(msg).toBe(
      "This conversation already has an unfinished goal — send a follow-up to continue it, or start a new conversation for a different goal.",
    );
    expect(msg).not.toContain("update_goal");
    expect(msg).not.toContain("clear it");
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
