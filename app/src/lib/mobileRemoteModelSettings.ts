import {
  CODING_MODELS,
  normalizeReasoningEffort,
  type ReasoningEffortId,
} from "./localAgent";
import type { CodeRemoteCommand } from "./mobileRemote";

export interface MobileRemoteModelSettings {
  model: string;
  reasoningEffort: ReasoningEffortId;
}

function requestPayload(command: CodeRemoteCommand): Record<string, unknown> | null {
  const value = command.request.payload;
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/** Validate the mobile selection against the same catalog the desktop picker
 * uses. A missing payload preserves the conversation's existing settings. */
export function mobileRemoteModelSettings(
  command: CodeRemoteCommand,
): MobileRemoteModelSettings | null {
  const payload = requestPayload(command);
  if (!payload) return null;
  const hasModel = Object.hasOwn(payload, "model");
  const hasEffort = Object.hasOwn(payload, "reasoning_effort");
  if (!hasModel && !hasEffort) return null;

  const rawModel = payload.model;
  if (typeof rawModel !== "string" || !rawModel.trim()) {
    throw new Error("The selected Clark Code model is invalid.");
  }
  const model = rawModel.trim();
  const config = CODING_MODELS.find((candidate) => candidate.id === model);
  if (!config) {
    throw new Error("The selected Clark Code model is not available on this desktop.");
  }

  // Older mobile clients may still send a user-selected effort. Ignore it so
  // every client uses the model's maximum supported reasoning level.
  return { model, reasoningEffort: normalizeReasoningEffort(model, config.defaultReasoningEffort) };
}
