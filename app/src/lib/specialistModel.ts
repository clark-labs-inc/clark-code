import {
  DEFAULT_LOCAL_SETTINGS,
  INCLUDED_CODING_MODEL_ID,
  SPECIALIST_MODEL_ID,
  SPECIALIST_REASONING_EFFORT,
  normalizeReasoningEffort,
} from "./localAgent";
import { specialistUsesIncludedModel, type SpecialistContext } from "./specialists";

export interface SpecialistModelSettings {
  model: string;
  reasoningEffort: string;
}

export function specialistModelSettings(
  context: SpecialistContext | null | undefined,
): SpecialistModelSettings | null {
  if (!context) return null;
  if (specialistUsesIncludedModel(context)) {
    const model = INCLUDED_CODING_MODEL_ID || DEFAULT_LOCAL_SETTINGS.model;
    return {
      model,
      reasoningEffort: normalizeReasoningEffort(model, DEFAULT_LOCAL_SETTINGS.reasoningEffort),
    };
  }
  return {
    model: SPECIALIST_MODEL_ID,
    reasoningEffort: SPECIALIST_REASONING_EFFORT,
  };
}
