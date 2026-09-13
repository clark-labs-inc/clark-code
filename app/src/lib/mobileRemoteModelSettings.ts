import { productModule } from "../product/productModule";
import { MobileRemoteFailure } from "./mobileRemoteFailure";
import {
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
    throw new MobileRemoteFailure("invalid_command", "The selected model is invalid.");
  }
  const model = rawModel.trim();
  const config = productModule().localAgent.models.find((candidate) => candidate.id === model);
  if (!config) {
    throw new MobileRemoteFailure("model_unavailable", "The selected model is no longer available on this computer. Choose a model again.");
  }

  // Older mobile clients may still send a user-selected effort. Ignore it so
  // every client uses the model's maximum supported reasoning level.
  return { model, reasoningEffort: config.defaultReasoningEffort };
}

export function desktopRemoteModels() {
  return productModule().localAgent.models.map((model) => ({
    id: model.id, label: model.label, reasoning_effort: model.defaultReasoningEffort,
  }));
}
