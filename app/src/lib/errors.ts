import type { RunOutcome } from "../core-bridge/types";
import { productModule } from "../product/productModule";

// Run failures use the typed provider contract below. `humanizeError` remains
// only for non-run errors and legacy records that predate `failure_kind`.

/** Native account-authority failures are deliberately distinct from ordinary
 * provider authentication errors. They mean the desktop's two process halves
 * no longer agree about which product account owns cloud state. */
export function isAccountReconnectError(raw?: string | null): boolean {
  const value = (raw ?? "").trim();
  return value ? productModule().errors.isAccountReconnectError(value) : false;
}

/** Map a typed terminal run failure to product language. */
export function humanizeRunFailure(
  outcome?: Pick<RunOutcome, "failure_kind" | "error">,
): string {
  switch (outcome?.failure_kind) {
    case "session_expired":
      return "Your the agent sign-in expired. Sign in again.";
    case "platform_key_rejected":
      return "Clark Code’s access key was rejected. Reconnect Clark Code and try again.";
    case "rate_limited":
      return "The model is busy right now (rate-limited). Give it a moment and try again.";
    case "transport_error":
      return "Couldn’t reach the model. Check your connection and try again.";
    case "provider_error":
      return "The model provider hit a temporary error. Please try again in a moment.";
    case "context_overflow":
      return "This conversation is too long for the model’s context window. Start a new session.";
    case "insufficient_credits":
      return "The selected provider’s usage limit has been reached.";
    case "tool_fatal":
      return "A coding action failed unexpectedly. Review the last step and try again.";
    case "local_state":
      return "Clark Code couldn’t continue this run. Start another run and try again.";
    case "iteration_limit":
      return "This run reached its step limit. Continue in this task to resume from the saved work.";
    case "runtime_interrupted":
      return "the agent restarted before this run finished. Continue from the saved history.";
    case "verification_incomplete":
      return "the agent finished its answer, but couldn’t independently verify one or more external changes. Review those actions before relying on them.";
    case "empty_response":
      return "The model returned no response. Please try again.";
    default:
      return "The run ended unexpectedly. Please try again.";
  }
}

/** Map a raw error string to a concise, human-readable message. */
export function humanizeError(raw?: string | null): string {
  const s = (raw ?? "").trim();
  if (!s) return "Something went wrong. Please try again.";
  const lower = s.toLowerCase();

  if (isAccountReconnectError(s)) {
    return "the agent needs to reconnect your account. Sign out and sign in again.";
  }

  // Defense in depth for older hosts or an accidentally reintroduced goal
  // admission error: internal lifecycle tools are never user instructions.
  if (lower.includes("unfinished goal already exists")) {
    return "This conversation already has an unfinished goal — send a follow-up to continue it, or start a new conversation for a different goal.";
  }

  // Native IPC/serde details are implementation diagnostics, not user-facing
  // failures. In particular, an older cloud snapshot can briefly expose an
  // enum-variant mismatch while it is being upgraded; never render that raw
  // payload in the conversation banner.
  if (
    lower.includes("invalid args")
    || lower.includes("unknown variant")
    || lower.includes("serde")
    || lower.includes("deserialization")
  ) {
    return "Clark Code couldn’t restore this conversation. Please try again.";
  }

  // Rate limited — the most common one, and what the screenshot showed.
  if (
    lower.includes("429") ||
    lower.includes("too many requests") ||
    lower.includes("rate-limited") ||
    lower.includes("rate limited") ||
    lower.includes("rate limit")
  ) {
    return "The model is busy right now (rate-limited). Give it a moment and try again.";
  }

  // Out of credits (normally handled by the upgrade prompt, but just in case).
  if (lower.includes("insufficient_credits") || lower.includes("out of credit")) {
    return "Clark Code’s usage limit has been reached.";
  }

  // Context window exceeded.
  if (
    lower.includes("context length") ||
    lower.includes("context_length") ||
    lower.includes("maximum context") ||
    lower.includes("too long") ||
    lower.includes("context window")
  ) {
    return "This conversation is too long for the model’s context window. Start a new session.";
  }

  // Cancelled by the user.
  if (lower.includes("cancel")) {
    return "The request was cancelled.";
  }

  // Network / timeout.
  if (
    lower.includes("timed out") ||
    lower.includes("timeout") ||
    lower.includes("connection") ||
    lower.includes("dns") ||
    lower.includes("network") ||
    lower.includes("request failed") ||
    lower.includes("failed to fetch")
  ) {
    return "Couldn’t reach the model. Check your connection and try again.";
  }

  // Any 5xx / generic provider error.
  if (
    /\b5\d\d\b/.test(s) ||
    lower.includes("provider returned error") ||
    lower.includes("internal server error") ||
    lower.includes("bad gateway") ||
    lower.includes("service unavailable") ||
    lower.includes("overloaded")
  ) {
    return "The model provider hit a temporary error. Please try again in a moment.";
  }

  // Unknown — pull a human sentence out of the noise (JSON blobs, status dumps)
  // rather than showing the raw payload.
  return cleanFallback(s);
}

/** Best-effort: extract a readable message from an arbitrary error string,
 *  preferring a human field inside any embedded JSON, then stripping JSON and
 *  truncating to a single short sentence. */
function cleanFallback(s: string): string {
  const fromJson = extractJsonMessage(s);
  let out = (fromJson ?? s).replace(/\{[\s\S]*\}/g, " ").replace(/\s+/g, " ").trim();
  // Drop a leading "model endpoint returned 500 Internal Server Error:" style prefix.
  out = out.replace(/^model (?:endpoint returned|request failed|stream error)[:\s-]*/i, "").trim();
  if (!out) return "Something went wrong. Please try again.";
  const firstSentence = out.split(/(?<=[.!?])\s/)[0];
  if (firstSentence.length >= 15) out = firstSentence;
  return out.length > 160 ? out.slice(0, 157).trimEnd() + "…" : out;
}

/** Find the most human-readable string field in any JSON embedded in `s`
 *  (a provider’s `metadata.raw`, `error.message`, or top-level `message`). */
function extractJsonMessage(s: string): string | null {
  const start = s.indexOf("{");
  const end = s.lastIndexOf("}");
  if (start === -1 || end <= start) return null;
  try {
    const obj = JSON.parse(s.slice(start, end + 1));
    const candidates = [
      obj?.error?.metadata?.raw,
      obj?.error?.message,
      obj?.message,
      obj?.error,
      obj?.detail,
    ];
    for (const c of candidates) {
      if (typeof c === "string" && c.trim()) return c.trim();
    }
  } catch {
    /* not valid JSON — fall through */
  }
  return null;
}
